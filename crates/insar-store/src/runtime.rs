use std::collections::BTreeSet;

use insar_core::{
    reduce_task_state, DownloadFailure, DownloadItemResult, DownloadOutcome, SnapshotItem,
    StoreAppendTaskRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskRuntimeSnapshot, TaskSelectionState, TaskStateCommand, TaskStateCommandKind,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::repository::{
    load_batch, load_compact_task_connection, load_explicit_task_source_item_ids,
    load_scheduler_task_connection, load_task_connection, sqlite_integer,
};
use crate::{StoreError, StoredTask, TaskStore};

#[derive(Debug, Clone, PartialEq)]
pub enum SchedulerCancelledFinish {
    Retried(TaskRuntimeSnapshot),
    Settled(Option<TaskRuntimeSnapshot>),
    Tombstoned,
}

pub(crate) const SCHEDULER_GENERATION_UNCERTAIN: &str = "SCHEDULER_GENERATION_UNCERTAIN";
pub(crate) const SCHEDULER_GENERATION_UNCERTAIN_MESSAGE: &str =
    "无法确认取消/重试代际，需重新检查任务记录";

pub(crate) fn scheduler_generation_uncertain_failure() -> DownloadFailure {
    DownloadFailure {
        code: SCHEDULER_GENERATION_UNCERTAIN.to_owned(),
        reason_zh: SCHEDULER_GENERATION_UNCERTAIN_MESSAGE.to_owned(),
        retryable: false,
    }
}

pub(crate) const SCHEMA_V38_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS scheduler_pending_cancelled_finishes (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    item_id TEXT NOT NULL CHECK(length(item_id) >= 1),
    started_revision INTEGER NOT NULL CHECK(started_revision >= 0),
    preserve_result INTEGER NOT NULL CHECK(preserve_result IN (0, 1)),
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    PRIMARY KEY(task_id, item_id)
) STRICT;
"#;

/// A cancelled worker result whose generation the scheduler has not settled.
/// The row is written only after a first settle attempt failed, and the
/// settle transaction removes it, so it never outlives its decision.
#[derive(Debug, Clone, PartialEq)]
pub struct SchedulerPendingCancelledFinish {
    pub result: DownloadItemResult,
    pub started_revision: u64,
    pub preserve_result: bool,
}

fn forget_pending_cancelled_finish(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<(), StoreError> {
    connection.execute(
        "DELETE FROM scheduler_pending_cancelled_finishes WHERE task_id = ?1 AND item_id = ?2",
        params![task_id, item_id],
    )?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancelledGeneration {
    Current,
    Retried,
    // Only a parsed, identity/revision-bound task-level command may be legacy.
    // Missing fields, damaged JSON and item-level empty commands remain errors.
    LegacyAmbiguous,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTaskEvent {
    pub event_id: String,
    pub task_id: String,
    pub sequence: u64,
    pub occurred_at: String,
    pub event_type: String,
    pub item_id: Option<String>,
    pub payload: Value,
    pub error: Option<Value>,
}

pub const DEFAULT_TASK_EVENT_PAGE_SIZE: u32 = 40;
pub const MAX_TASK_EVENT_PAGE_SIZE: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReacquireEventStatus {
    Committed,
    Conflict,
    Unknown,
}

/// Identity-only event lookup. An absent event does not establish that a
/// writer has finished, and events may not survive future physical pruning.
pub(crate) fn reacquire_event_identity_status(
    connection: &Connection,
    request: &StoreUpdateTaskRequest,
) -> Result<ReacquireEventStatus, StoreError> {
    request.validate().map_err(StoreError::invalid)?;
    if request.command.kind != TaskStateCommandKind::ReacquireItems
        || request.occurred_at.trim().is_empty()
    {
        return Err(StoreError::invalid("reacquire event request is invalid"));
    }
    let encoded = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
    let event = connection.query_row(
        "SELECT task_id, event_type, payload_json, occurred_at, sequence FROM task_events WHERE event_id = ?1",
        [&request.event_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?)),
    ).optional()?;
    Ok(match event {
        None => ReacquireEventStatus::Unknown,
        Some((task_id, event_type, payload, occurred_at, sequence))
            if task_id == request.task_id
                && event_type == "reacquire_items"
                && (payload == encoded
                    || task_event_request_matches_stamped_payload(&payload, request))
                && occurred_at == request.occurred_at
                && u64::try_from(sequence).ok() == request.expected_revision.checked_add(1)
                && request.expected_revision.checked_add(1).is_some() =>
        {
            ReacquireEventStatus::Committed
        }
        Some(_) => ReacquireEventStatus::Conflict,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoreAppendExplicitTaskItemsRequest {
    pub task_id: String,
    pub expected_revision: u64,
    pub event_id: String,
    pub occurred_at: String,
    pub source_item_ids: Vec<String>,
    pub task_items: Vec<SnapshotItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreTaskEventPageRequest {
    pub task_id: String,
    pub expected_revision: u64,
    pub before_sequence: Option<u64>,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskEventPageItem {
    pub sequence: u64,
    pub occurred_at: String,
    pub level: String,
    pub event_kind: String,
    pub item_label: Option<String>,
    pub code: Option<String>,
    pub message_zh: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskEventPage {
    pub task_id: String,
    pub revision: u64,
    pub page_size: u32,
    pub has_more: bool,
    pub next_before_sequence: Option<u64>,
    pub items: Vec<StoreTaskEventPageItem>,
}

impl TaskStore {
    /// Check an exact reacquire event before mutable service admission checks.
    /// Unknown does not prove that an earlier request has stopped executing.
    pub fn reacquire_event_status(
        &self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<crate::ReacquireEventStatus, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let receipt = crate::source_receipts::reacquire_receipt_status(&transaction, request)?;
        let status = if receipt == ReacquireEventStatus::Unknown {
            reacquire_event_identity_status(&transaction, request)?
        } else {
            receipt
        };
        if status == ReacquireEventStatus::Committed {
            reject_tombstoned_task(&transaction, &request.task_id)?;
            if !crate::core_restore::task_is_actionable(&transaction, &request.task_id)?
                || load_compact_task_connection(&transaction, &request.task_id)?.is_none()
            {
                return Err(StoreError::NotFound);
            }
        }
        transaction.commit()?;
        Ok(status)
    }

    pub fn append_explicit_task_items(
        &mut self,
        request: &StoreAppendExplicitTaskItemsRequest,
    ) -> Result<StoredTask, StoreError> {
        self.append_explicit_task_items_inner(request, None, None, None, None)
    }

    /// Internal service boundary for a progressive Burst task. The caller
    /// freezes assets from these parent rows; the transaction rechecks the
    /// complete evidence against the sealed original batch before writing.
    pub fn append_progressive_burst_items(
        &mut self,
        request: &StoreAppendExplicitTaskItemsRequest,
        source_parents: &[SnapshotItem],
    ) -> Result<StoredTask, StoreError> {
        self.append_explicit_task_items_inner(request, Some(source_parents), None, None, None)
    }

    /// Burst requests preserve the complete external identity in the same
    /// transaction as membership and event writes.
    pub fn append_burst_items_with_receipt(
        &mut self,
        request: &StoreAppendExplicitTaskItemsRequest,
        source_parents: Option<&[SnapshotItem]>,
        identity: &crate::StoreBurstAppendIdentity,
    ) -> Result<StoredTask, StoreError> {
        let selected: BTreeSet<_> = identity.selected_product_ids.iter().collect();
        let sources: BTreeSet<_> = request.source_item_ids.iter().collect();
        if identity.request_id.trim().is_empty()
            || selected.is_empty()
            || selected.len() != identity.selected_product_ids.len()
            || sources.len() != request.source_item_ids.len()
            || selected != sources
            || identity.task_id != request.task_id
            || identity.expected_task_revision != request.expected_revision
            || identity.event_id != request.event_id
            || identity.occurred_at != request.occurred_at
        {
            return Err(StoreError::invalid(
                "Burst append identity differs from request",
            ));
        }
        self.append_explicit_task_items_inner(request, source_parents, Some(identity), None, None)
    }

    /// The service built NISAR items from this exact observed authority. Recheck
    /// that authority inside the write transaction before accepting the items.
    /// This does not replace the service's provider-specific item builder.
    pub fn append_nisar_items_with_authority(
        &mut self,
        request: &StoreAppendExplicitTaskItemsRequest,
        observed_task: &StoredTask,
        observed_batch: &insar_core::FrozenSearchBatch,
        identity: &crate::StoreNisarAppendIdentity,
    ) -> Result<StoredTask, StoreError> {
        if identity.task_id != request.task_id
            || identity.expected_task_revision != request.expected_revision
            || identity.source_item_ids != request.source_item_ids
            || identity.event_id != request.event_id
            || identity.occurred_at != request.occurred_at
            || observed_task.runtime.task_id != request.task_id
            || observed_task.runtime.revision != request.expected_revision
            || observed_task.snapshot.batch_id != observed_batch.batch_id
        {
            return Err(StoreError::invalid(
                "NISAR append authority differs from request",
            ));
        }
        self.append_explicit_task_items_inner(
            request,
            None,
            None,
            Some((observed_task, observed_batch)),
            Some(identity),
        )
    }

    fn append_explicit_task_items_inner(
        &mut self,
        request: &StoreAppendExplicitTaskItemsRequest,
        source_parents: Option<&[SnapshotItem]>,
        receipt: Option<&crate::StoreBurstAppendIdentity>,
        nisar_authority: Option<(&StoredTask, &insar_core::FrozenSearchBatch)>,
        nisar_receipt: Option<&crate::StoreNisarAppendIdentity>,
    ) -> Result<StoredTask, StoreError> {
        if let Some(parents) = source_parents {
            if parents.is_empty()
                || parents.len() > crate::MAX_WORKBENCH_PAGE_SIZE as usize
                || request.source_item_ids.len() != parents.len()
                || request
                    .source_item_ids
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != parents.len()
                || request.task_items.is_empty()
                || request.task_items.len() > parents.len() * 2
            {
                return Err(StoreError::invalid(
                    "progressive Burst selection is invalid",
                ));
            }
        }
        if !valid_bounded_scalar(&request.task_id, 128)
            || !valid_bounded_scalar(&request.event_id, 192)
            || request.occurred_at.trim().is_empty()
            || request.source_item_ids.is_empty()
            || request.task_items.is_empty()
        {
            return Err(StoreError::invalid(
                "explicit task append request is invalid",
            ));
        }
        let request_json = match source_parents {
            Some(parents) => serde_json::to_string(&(request, parents)),
            None => serde_json::to_string(request),
        }
        .map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(identity) = nisar_receipt {
            match crate::source_receipts::nisar_append_status(&transaction, identity)? {
                crate::NisarAppendReceiptStatus::Committed => {
                    reject_tombstoned_task(&transaction, &request.task_id)?;
                    if !crate::core_restore::task_is_actionable(&transaction, &request.task_id)? {
                        return Err(StoreError::NotFound);
                    }
                    let task = load_task_connection(&transaction, &request.task_id)?
                        .ok_or(StoreError::NotFound)?;
                    if task.runtime.revision <= identity.expected_task_revision
                        || task.runtime.selected_item_ids.len() != task.snapshot.candidates.len()
                    {
                        return Err(StoreError::invalid("NISAR append target is inconsistent"));
                    }
                    transaction.commit()?;
                    return Ok(task);
                }
                crate::NisarAppendReceiptStatus::Conflict => return Err(StoreError::AlreadyExists),
                crate::NisarAppendReceiptStatus::Unknown => {}
            }
        }
        if let Some((observed_task, observed_batch)) = nisar_authority {
            reject_tombstoned_task(&transaction, &request.task_id)?;
            let current = load_task_connection(&transaction, &request.task_id)?
                .ok_or(StoreError::NotFound)?;
            let current_batch =
                load_batch(&transaction, &observed_batch.batch_id)?.ok_or(StoreError::NotFound)?;
            if current.runtime.revision != observed_task.runtime.revision
                || current.snapshot != observed_task.snapshot
                || current_batch != *observed_batch
            {
                return Err(StoreError::RevisionConflict);
            }
        }
        if let Some(identity) = receipt {
            match crate::source_receipts::burst_append_status(&transaction, identity)? {
                crate::BurstAppendReceiptStatus::Committed => {
                    reject_tombstoned_task(&transaction, &request.task_id)?;
                    let task = load_task_connection(&transaction, &request.task_id)?
                        .ok_or(StoreError::NotFound)?;
                    transaction.commit()?;
                    return Ok(task);
                }
                crate::BurstAppendReceiptStatus::Conflict => return Err(StoreError::AlreadyExists),
                crate::BurstAppendReceiptStatus::Unknown => {}
            }
        }
        if let Some((task_id, payload_json, event_type)) = transaction
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
            .optional()?
        {
            if task_id != request.task_id
                || payload_json != request_json
                || event_type != "append_items"
            {
                return Err(StoreError::AlreadyExists);
            }
            let task = load_scheduler_task_connection(&transaction, &request.task_id)?
                .ok_or(StoreError::NotFound)?;
            transaction.commit()?;
            return Ok(task);
        }
        reject_closed_task(&transaction, &request.task_id)?;
        if !crate::core_restore::task_is_actionable(&transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        if load_explicit_task_source_item_ids(&transaction, &request.task_id)?.is_none() {
            return Err(StoreError::invalid(
                "explicit item append requires an explicit-item task",
            ));
        }
        let mut task = load_scheduler_task_connection(&transaction, &request.task_id)?
            .ok_or(StoreError::NotFound)?;
        if task.runtime.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if receipt.is_some()
            && source_parents.is_none()
            && crate::loading_admission::verified_progressive_source(&transaction, &task.snapshot)?
                .is_some()
        {
            return Err(StoreError::invalid(
                "progressive Burst source evidence is required",
            ));
        }
        let batch_items = if let Some(parents) = source_parents {
            if parents.is_empty() || parents.len() > crate::MAX_WORKBENCH_PAGE_SIZE as usize {
                return Err(StoreError::invalid(
                    "progressive Burst source selection is invalid",
                ));
            }
            let source_id = crate::loading_admission::verified_progressive_source(
                &transaction,
                &task.snapshot,
            )?
            .ok_or_else(|| StoreError::invalid("progressive Burst source is required"))?;
            let rows = crate::batch_candidates::load_batch_candidate_items_connection(
                &transaction,
                &source_id,
                &request.source_item_ids,
            )?
            .ok_or(StoreError::NotFound)?;
            let evidence = parents
                .iter()
                .map(|item| (item.item_id.as_str(), item))
                .collect::<std::collections::HashMap<_, _>>();
            if evidence.len() != parents.len()
                || rows.len() != parents.len()
                || rows.iter().any(|item| {
                    item.metadata.contains_key("source_item_id")
                        || evidence.get(item.item_id.as_str()).copied() != Some(item)
                })
            {
                return Err(StoreError::invalid(
                    "progressive Burst source evidence differs",
                ));
            }
            let requested =
                serde_json::to_string(&request.source_item_ids).map_err(StoreError::Corrupt)?;
            let conflict: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM batch_task_items i JOIN task_runtime r ON r.task_id = i.task_id \
                 LEFT JOIN task_tombstones d ON d.task_id = i.task_id \
                 WHERE i.source_batch_id = ?1 AND i.source_item_id IN (SELECT value FROM json_each(?2)) \
                 AND i.task_id != ?3 AND d.task_id IS NULL \
                 AND r.status IN ('queued','running','paused','interrupted'))",
                params![source_id, requested, task.runtime.task_id], |row| row.get(0),
            )?;
            if conflict {
                return Err(StoreError::PartialActiveTaskOverlap);
            }
            insar_core::validate_burst_task_items(
                &rows,
                &task.snapshot.candidates,
                &request.task_items,
            )
            .map_err(|_| {
                StoreError::invalid("progressive Burst assets differ from source or policy")
            })?;
            rows
        } else {
            load_batch(&transaction, &task.snapshot.batch_id)?
                .ok_or_else(|| StoreError::invalid("task source batch does not exist"))?
                .candidates
        };
        let batch_ids = batch_items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<BTreeSet<_>>();
        let source_ids = request
            .source_item_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if receipt.is_some() && source_parents.is_none() {
            let requested =
                serde_json::to_string(&request.source_item_ids).map_err(StoreError::Corrupt)?;
            let conflict: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM batch_task_items i JOIN task_runtime r ON r.task_id = i.task_id \
                 LEFT JOIN task_tombstones d ON d.task_id = i.task_id WHERE i.source_batch_id = ?1 \
                 AND i.source_item_id IN (SELECT value FROM json_each(?2)) AND i.task_id != ?3 AND d.task_id IS NULL \
                 AND r.status IN ('queued','running','paused','interrupted'))",
                params![task.snapshot.batch_id, requested, request.task_id], |row| row.get(0),
            )?;
            if conflict {
                return Err(StoreError::PartialActiveTaskOverlap);
            }
            let parents = batch_items
                .iter()
                .filter(|item| source_ids.contains(item.item_id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            insar_core::validate_burst_task_items(
                &parents,
                &task.snapshot.candidates,
                &request.task_items,
            )
            .map_err(|_| StoreError::invalid("Burst assets differ from source or policy"))?;
        }
        let existing_sources = task
            .snapshot
            .candidates
            .iter()
            .filter_map(|item| item.metadata.get("source_item_id"))
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        if source_ids.len() != request.source_item_ids.len()
            || !source_ids.is_subset(&batch_ids)
            || !source_ids.is_disjoint(&existing_sources)
        {
            return Err(StoreError::invalid(
                "explicit append source products are invalid or already selected",
            ));
        }
        let item_ids = request
            .task_items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>();
        let unique_item_ids = item_ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let existing_item_ids = task
            .snapshot
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut referenced_sources = BTreeSet::new();
        if unique_item_ids.len() != request.task_items.len()
            || !unique_item_ids.is_disjoint(&existing_item_ids)
            || request
                .task_items
                .iter()
                .any(|item| item.provider_id != task.snapshot.provider_id)
        {
            return Err(StoreError::invalid(
                "explicit appended task items are invalid",
            ));
        }
        for item in &request.task_items {
            let source_item_id = item
                .metadata
                .get("source_item_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    StoreError::invalid("explicit appended item lacks its source item ID")
                })?;
            if !source_ids.contains(source_item_id) {
                return Err(StoreError::invalid(
                    "explicit appended item refers outside the appended source products",
                ));
            }
            referenced_sources.insert(source_item_id);
        }
        if referenced_sources != source_ids {
            return Err(StoreError::invalid(
                "explicit appended source products are not represented exactly",
            ));
        }
        task.snapshot
            .candidates
            .extend(request.task_items.iter().cloned());
        task.snapshot.validate().map_err(StoreError::invalid)?;
        let task = append_task_membership_in_transaction(
            &transaction,
            task,
            request.expected_revision,
            &item_ids,
            &request.event_id,
            &request.occurred_at,
            &request_json,
        )?;
        if let Some(identity) = receipt {
            crate::source_receipts::insert_burst_append_receipt(
                &transaction,
                identity,
                &request_json,
                task.runtime.revision,
            )?;
        }
        if let Some(identity) = nisar_receipt {
            if task.runtime.revision
                != identity
                    .expected_task_revision
                    .checked_add(1)
                    .ok_or_else(|| StoreError::invalid("NISAR append revision overflow"))?
                || task.runtime.selected_item_ids.len() != task.snapshot.candidates.len()
            {
                return Err(StoreError::invalid("NISAR append target is inconsistent"));
            }
            crate::source_receipts::insert_nisar_append_receipt(
                &transaction,
                identity,
                &request_json,
                task.runtime.revision,
            )?;
        }
        transaction.commit()?;
        Ok(task)
    }

    pub fn append_task_items(
        &mut self,
        request: &StoreAppendTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.item_ids.is_empty() || request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid(
                "append requires item IDs and an event timestamp",
            ));
        }
        let appended: BTreeSet<&str> = request.item_ids.iter().map(String::as_str).collect();
        if appended.len() != request.item_ids.len() {
            return Err(StoreError::invalid("append item IDs must be unique"));
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
        reject_closed_task(&transaction, &request.task_id)?;
        if !crate::core_restore::task_is_actionable(&transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        let mut task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        if task.runtime.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        task = append_task_membership_in_transaction(
            &transaction,
            task,
            request.expected_revision,
            &request.item_ids,
            &request.event_id,
            &request.occurred_at,
            &request_json,
        )?;
        transaction.commit()?;
        Ok(task)
    }

    pub fn is_task_explicit(&self, task_id: &str) -> Result<bool, StoreError> {
        Ok(
            crate::repository::load_explicit_task_source_item_ids(&self.connection, task_id)?
                .is_some(),
        )
    }

    pub fn update_task(
        &mut self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        // Reacquire must share the transaction that clears obsolete orbit gates.
        if request.command.kind == TaskStateCommandKind::ReacquireItems {
            return self.update_task_and_clear_orbit_rechecks(request);
        }
        request.validate().map_err(StoreError::invalid)?;
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("event timestamp is required"));
        }
        let request_json = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = update_task_in_transaction(&transaction, request, &request_json)?;
        transaction.commit()?;
        Ok(task)
    }

    /// Retry specific failed or cancelled items on an archived History task,
    /// unarchiving the task in the same transaction without modifying `update_task`
    /// semantics for unarchived vs archived command validation.
    pub fn retry_archived_task_items(
        &mut self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.command.kind != TaskStateCommandKind::RetryItems {
            return Err(StoreError::invalid(
                "archived item retry requires RetryItems command",
            ));
        }
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("event timestamp is required"));
        }
        let request_json = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task =
            update_task_in_transaction_with_options(&transaction, request, &request_json, true)?;
        transaction.commit()?;
        Ok(task)
    }

    /// Resume one or more paused items of a task; if the whole task was paused
    /// (`task_paused == true`), un-pauses ONLY the requested items by moving all
    /// other non-terminal items into `paused_item_ids` and clearing `task_paused`.
    pub fn resume_paused_task_items(
        &mut self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.command.kind != TaskStateCommandKind::ResumeItems {
            return Err(StoreError::invalid(
                "item resume requires ResumeItems command",
            ));
        }
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("event timestamp is required"));
        }
        let request_json = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT task_id FROM task_events WHERE event_id = ?1",
                [&request.event_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            let task = update_task_in_transaction(&transaction, request, &request_json)?;
            transaction.commit()?;
            return Ok(task);
        }
        reject_closed_task(&transaction, &request.task_id)?;
        if !crate::core_restore::task_is_actionable(&transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        let mut task = load_scheduler_task_connection(&transaction, &request.task_id)?
            .ok_or(StoreError::NotFound)?;
        if task.runtime.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if !task.runtime.task_paused {
            let task = update_task_in_transaction(&transaction, request, &request_json)?;
            transaction.commit()?;
            return Ok(task);
        }
        if task.snapshot.operation.as_ref().is_some_and(|operation| {
            operation.kind == "gacos"
                && operation.approved_worker_operation == "gacos_manual_request"
        }) {
            return Err(StoreError::GacosManualActionRequired);
        }
        let mut state = selection_state(&task)?;
        let resume_set: BTreeSet<String> = request.command.item_ids.iter().cloned().collect();
        if resume_set.is_empty() {
            return Err(StoreError::invalid("task command requires item IDs"));
        }
        for item_id in &resume_set {
            if !state.selected_ids.contains(item_id) {
                return Err(StoreError::invalid(format!(
                    "item {item_id} is not selected in task {}",
                    request.task_id
                )));
            }
            if state.completed_ids.contains(item_id)
                || state.failed_ids.contains(item_id)
                || state.skipped_ids.contains(item_id)
                || state.cancelled_ids.contains(item_id)
            {
                return Err(StoreError::invalid(format!(
                    "terminal item {item_id} cannot be resumed"
                )));
            }
        }
        let other_active: Vec<String> = state
            .active_ids
            .iter()
            .filter(|id| !resume_set.contains(*id))
            .cloned()
            .collect();
        let other_queued: Vec<String> = state
            .queued_ids
            .iter()
            .filter(|id| !resume_set.contains(*id))
            .cloned()
            .collect();
        for id in other_active {
            state.active_ids.remove(&id);
            state.paused_ids.insert(id);
        }
        for id in other_queued {
            state.queued_ids.remove(&id);
            state.paused_ids.insert(id);
        }
        for item_id in &resume_set {
            state.paused_ids.remove(item_id);
            state.active_ids.remove(item_id);
            state.queued_ids.insert(item_id.clone());
        }
        state.task_paused = false;
        insar_core::validate_task_state(&state).map_err(|error| {
            StoreError::invalid(format!("{}: {}", error.code, error.user_message))
        })?;
        let revision = request
            .expected_revision
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
        apply_selection_state(&mut task.runtime, state);
        task.runtime.revision = revision;
        task.runtime.updated_at = request.occurred_at.clone();
        task.runtime.status = lifecycle_status(&task.runtime);
        task.runtime.validate().map_err(StoreError::invalid)?;
        let revision_sql = sqlite_integer(revision, "task revision")?;
        let expected_sql = sqlite_integer(request.expected_revision, "expected task revision")?;
        let runtime_json = serde_json::to_string(&task.runtime).map_err(StoreError::Corrupt)?;
        let changed = transaction.execute(
            "UPDATE task_runtime SET status = ?1, revision = ?2, updated_at = ?3, state_json = ?4 \
             WHERE task_id = ?5 AND revision = ?6",
            params![
                lifecycle_status_name(&task.runtime.status),
                revision_sql,
                task.runtime.updated_at,
                runtime_json,
                task.runtime.task_id,
                expected_sql
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        transaction.execute(
            "INSERT INTO task_events( \
                event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                request.event_id,
                request.task_id,
                revision_sql,
                request.occurred_at,
                command_name(&request.command.kind),
                single_item_id(&request.command),
                request_json
            ],
        )?;
        transaction.commit()?;
        Ok(task)
    }

    /// Scheduler-only transition path for already-admitted live tasks.
    ///
    /// These commands never change frozen task membership or product metadata,
    /// so the scheduler can update the canonical runtime row without
    /// deserializing the immutable task snapshot on every item transition.
    pub fn update_scheduler_runtime(
        &mut self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<TaskRuntimeSnapshot, StoreError> {
        self.update_scheduler_runtime_inner(request, None)
    }

    /// Finish a pre-I/O migration rejection as a terminal, visible local failure.
    ///
    /// In one IMMEDIATE transaction the exact download result/progress rows the
    /// visible failure replaces are retained as immutable refusal evidence, the
    /// failure is persisted with ordinary result semantics, and the item is marked
    /// failed with the diagnostic on its task event. Checkpoints, anchors and
    /// proofs are untouched. A concurrent pause or cancellation still wins: an
    /// item that is no longer active, or a task-level pause, replaces nothing.
    pub fn finish_nisar_budget_upgrade_rejection(
        &mut self,
        request: &StoreUpdateTaskRequest,
        result: &insar_core::DownloadItemResult,
    ) -> Result<TaskRuntimeSnapshot, StoreError> {
        validate_budget_upgrade_rejection(request, result)?;
        self.update_scheduler_runtime_inner(request, Some((RejectionEvidence::Retained, result)))
    }

    /// Finish a pre-I/O migration rejection whose refusal evidence could not be
    /// retained (`finish_nisar_budget_upgrade_rejection` failed for a reason
    /// other than a revision conflict). Review 2026-10-17 round 2, P2-1:
    /// nothing is overwritten. An existing download result or progress row
    /// stays exactly as it is and the diagnostic is kept on the item's task
    /// event; the visible result is written only when the item has no download
    /// row at all, since nothing is replaced then. A concurrent pause or
    /// cancellation still wins exactly as in the retained path.
    pub fn finish_nisar_budget_upgrade_rejection_unretained(
        &mut self,
        request: &StoreUpdateTaskRequest,
        result: &insar_core::DownloadItemResult,
    ) -> Result<TaskRuntimeSnapshot, StoreError> {
        validate_budget_upgrade_rejection(request, result)?;
        self.update_scheduler_runtime_inner(request, Some((RejectionEvidence::Unretained, result)))
    }

    fn update_scheduler_runtime_inner(
        &mut self,
        request: &StoreUpdateTaskRequest,
        rejection: Option<(RejectionEvidence, &insar_core::DownloadItemResult)>,
    ) -> Result<TaskRuntimeSnapshot, StoreError> {
        let mut request = request.clone();
        request.validate().map_err(StoreError::invalid)?;
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("event timestamp is required"));
        }
        if !matches!(
            request.command.kind,
            TaskStateCommandKind::ActivateItems
                | TaskStateCommandKind::PauseItems
                | TaskStateCommandKind::MarkCompleted
                | TaskStateCommandKind::MarkFailed
                | TaskStateCommandKind::MarkSkipped
                | TaskStateCommandKind::CancelItems
        ) {
            return Err(StoreError::invalid(
                "scheduler runtime path does not accept this task command",
            ));
        }

        let error_json = rejection
            .and_then(|(_, result)| result.failure.as_ref())
            .map(serde_json::to_string)
            .transpose()
            .map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((task_id, payload_json, stored_error, event_type)) = transaction
            .query_row(
                "SELECT task_id, payload_json, error_json, event_type FROM task_events WHERE event_id = ?1",
                [&request.event_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?)),
            )
            .optional()?
        {
            // A task-level pause changes the recorded command atomically below.
            // Replay still compares the exact effective request and diagnostic.
            if rejection.is_some() && event_type == command_name(&TaskStateCommandKind::PauseItems) {
                request.command.kind = TaskStateCommandKind::PauseItems;
            }
            let request_json = serde_json::to_string(&request).map_err(StoreError::Corrupt)?;
            if task_id != request.task_id || payload_json != request_json || stored_error != error_json {
                return Err(StoreError::AlreadyExists);
            }
            if let Some((mode, result)) = rejection {
                // A retained failed refusal recorded its evidence; a pause
                // conversion and an unretained refusal did not.
                let evidence = crate::repository::nisar_budget_upgrade::refusal_recorded_for_event(
                    &transaction,
                    &request.event_id,
                )?;
                let failed = event_type == command_name(&TaskStateCommandKind::MarkFailed);
                let retained = failed && mode == RejectionEvidence::Retained;
                if evidence.is_some() != retained
                    || evidence
                        .as_ref()
                        .is_some_and(|evidence| Some(evidence.failure()) != result.failure.as_ref())
                {
                    return Err(StoreError::AlreadyExists);
                }
            }
            let runtime = load_runtime_snapshot_connection(&transaction, &request.task_id)?
                .ok_or(StoreError::NotFound)?;
            transaction.commit()?;
            return Ok(runtime);
        }

        let mut runtime = load_runtime_snapshot_connection(&transaction, &request.task_id)?
            .ok_or(StoreError::NotFound)?;
        if rejection.is_some() {
            let item_id = &request.command.item_ids[0];
            if !runtime.selected_item_ids.contains(item_id) {
                return Err(StoreError::invalid(
                    "migration rejection item is not selected",
                ));
            }
            if !runtime.active_item_ids.contains(item_id) {
                transaction.commit()?;
                return Ok(runtime);
            }
            if runtime.task_paused {
                request.command.kind = TaskStateCommandKind::PauseItems;
            }
        }
        reject_closed_task(&transaction, &request.task_id)?;
        reject_gacos_manual_scheduler_command(
            &transaction,
            &request.task_id,
            &request.command.kind,
        )?;
        if runtime.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }

        let request_json = serde_json::to_string(&request).map_err(StoreError::Corrupt)?;
        if let Some((mode, result)) = rejection {
            if request.command.kind == TaskStateCommandKind::MarkFailed {
                match mode {
                    RejectionEvidence::Retained => {
                        crate::repository::nisar_budget_upgrade::record_refusal_and_visible_result(
                            &transaction,
                            &request.event_id,
                            &request.occurred_at,
                            result,
                        )?;
                    }
                    RejectionEvidence::Unretained => {
                        crate::repository::nisar_budget_upgrade::show_unretained_rejection(
                            &transaction,
                            result,
                        )?;
                    }
                }
            }
        }
        let command = store_command(&transaction, &request.task_id, &request.command)?;
        let state = selection_state_from_runtime(&runtime)?;
        let next = reduce_task_state(&state, &command).map_err(|error| {
            StoreError::invalid(format!("{}: {}", error.code, error.user_message))
        })?;
        let revision = request
            .expected_revision
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
        apply_selection_state(&mut runtime, next);
        runtime.revision = revision;
        runtime.updated_at = request.occurred_at.clone();
        runtime.status = lifecycle_status(&runtime);
        runtime.validate().map_err(StoreError::invalid)?;

        let revision_sql = sqlite_integer(revision, "task revision")?;
        let expected_sql = sqlite_integer(request.expected_revision, "expected task revision")?;
        let runtime_json = serde_json::to_string(&runtime).map_err(StoreError::Corrupt)?;
        let changed = transaction.execute(
            "UPDATE task_runtime SET status = ?1, revision = ?2, updated_at = ?3, state_json = ?4 \
             WHERE task_id = ?5 AND revision = ?6",
            params![
                lifecycle_status_name(&runtime.status),
                revision_sql,
                runtime.updated_at,
                runtime_json,
                runtime.task_id,
                expected_sql
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        transaction.execute(
            "INSERT INTO task_events( \
                event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                request.event_id,
                request.task_id,
                revision_sql,
                request.occurred_at,
                command_name(&request.command.kind),
                single_item_id(&request.command),
                request_json,
                error_json
            ],
        )?;
        transaction.commit()?;
        Ok(runtime)
    }
    /// Confirm deletion without reading or mutating the task runtime. A
    /// missing row or a read error is not evidence of deletion.
    pub fn confirm_scheduler_task_tombstone(&mut self, task_id: &str) -> Result<bool, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let tombstoned = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [task_id],
            |row| row.get(0),
        )?;
        transaction.commit()?;
        Ok(tombstoned)
    }

    /// Keeps a cancelled result the scheduler could not settle, so a restart
    /// does not lose it. A later record for the same item replaces it.
    pub fn record_scheduler_pending_cancelled_finish(
        &mut self,
        pending: &SchedulerPendingCancelledFinish,
    ) -> Result<(), StoreError> {
        pending.result.validate().map_err(StoreError::invalid)?;
        if pending.result.outcome != DownloadOutcome::Cancelled {
            return Err(StoreError::invalid(
                "a pending scheduler finish must be a cancelled result",
            ));
        }
        let result_json = serde_json::to_string(&pending.result).map_err(StoreError::Corrupt)?;
        self.connection.execute(
            "INSERT INTO scheduler_pending_cancelled_finishes(task_id, item_id, started_revision, preserve_result, result_json) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(task_id, item_id) DO UPDATE SET started_revision = excluded.started_revision, \
             preserve_result = excluded.preserve_result, result_json = excluded.result_json",
            params![
                pending.result.task_id,
                pending.result.item_id,
                sqlite_integer(pending.started_revision, "worker start revision")?,
                i64::from(pending.preserve_result),
                result_json,
            ],
        )?;
        Ok(())
    }

    /// Every cancelled result left unsettled by an earlier scheduler run.
    pub fn load_scheduler_pending_cancelled_finishes(
        &self,
    ) -> Result<Vec<SchedulerPendingCancelledFinish>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT started_revision, preserve_result, result_json \
             FROM scheduler_pending_cancelled_finishes ORDER BY task_id, item_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (started_revision, preserve_result, result_json) = row?;
            let result: DownloadItemResult =
                serde_json::from_str(&result_json).map_err(StoreError::Corrupt)?;
            result.validate().map_err(StoreError::invalid)?;
            Ok(SchedulerPendingCancelledFinish {
                result,
                started_revision: u64::try_from(started_revision)
                    .map_err(|_| StoreError::invalid("negative worker start revision"))?,
                preserve_result: preserve_result != 0,
            })
        })
        .collect()
    }

    /// Drops one pending cancelled result without settling it.
    pub fn forget_scheduler_pending_cancelled_finish(
        &mut self,
        task_id: &str,
        item_id: &str,
    ) -> Result<(), StoreError> {
        forget_pending_cancelled_finish(&self.connection, task_id, item_id)
    }

    /// Strict scheduler finish path. Legacy task-level payloads without the
    /// stamped item membership remain an error for callers that require proof.
    pub fn finish_scheduler_cancelled_result(
        &mut self,
        result: &DownloadItemResult,
        started_revision: u64,
        expected_revision: u64,
        event_id: &str,
        preserve_result: bool,
    ) -> Result<SchedulerCancelledFinish, StoreError> {
        self.finish_scheduler_cancelled_result_inner(
            result,
            started_revision,
            expected_revision,
            event_id,
            preserve_result,
            false,
        )
    }

    /// Scheduler-only recovery for a valid legacy task-level cancel/retry
    /// history whose item membership cannot be proven. The entire decision is
    /// fenced and committed in one transaction; no download result/progress
    /// row is touched. A fresh stamped cancel/retry pair wins over an older
    /// legacy ambiguity, while malformed evidence remains an error.
    pub fn finish_scheduler_cancelled_result_with_legacy_policy(
        &mut self,
        result: &DownloadItemResult,
        started_revision: u64,
        expected_revision: u64,
        event_id: &str,
        preserve_result: bool,
    ) -> Result<SchedulerCancelledFinish, StoreError> {
        self.finish_scheduler_cancelled_result_inner(
            result,
            started_revision,
            expected_revision,
            event_id,
            preserve_result,
            true,
        )
    }

    fn finish_scheduler_cancelled_result_inner(
        &mut self,
        result: &DownloadItemResult,
        started_revision: u64,
        expected_revision: u64,
        event_id: &str,
        preserve_result: bool,
        allow_legacy_failure: bool,
    ) -> Result<SchedulerCancelledFinish, StoreError> {
        result.validate().map_err(StoreError::invalid)?;
        if result.outcome != DownloadOutcome::Cancelled {
            return Err(StoreError::invalid(
                "scheduler cancelled finish requires cancelled result",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let tombstoned: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [&result.task_id],
            |row| row.get(0),
        )?;
        if tombstoned {
            forget_pending_cancelled_finish(&transaction, &result.task_id, &result.item_id)?;
            transaction.commit()?;
            return Ok(SchedulerCancelledFinish::Tombstoned);
        }
        let runtime = load_runtime_snapshot_connection(&transaction, &result.task_id)?
            .ok_or(StoreError::NotFound)?;
        if runtime.revision != expected_revision || started_revision > runtime.revision {
            return Err(StoreError::RevisionConflict);
        }
        if !runtime.selected_item_ids.contains(&result.item_id) {
            return Err(StoreError::invalid("scheduler result item is not selected"));
        }
        let generation = classify_cancelled_generation(
            &transaction,
            &runtime,
            &result.item_id,
            started_revision,
        )?;
        if generation == CancelledGeneration::Retried {
            forget_pending_cancelled_finish(&transaction, &result.task_id, &result.item_id)?;
            transaction.commit()?;
            return Ok(SchedulerCancelledFinish::Retried(runtime));
        }
        let legacy_failure = generation == CancelledGeneration::LegacyAmbiguous;
        if legacy_failure && !allow_legacy_failure {
            return Err(StoreError::invalid(
                "legacy task-level cancellation evidence lacks item membership",
            ));
        }
        let terminal = runtime.completed_item_ids.contains(&result.item_id)
            || runtime.failed_item_ids.contains(&result.item_id)
            || runtime.skipped_item_ids.contains(&result.item_id)
            || runtime.cancelled_item_ids.contains(&result.item_id);
        if terminal {
            if runtime.cancelled_item_ids.contains(&result.item_id)
                && !preserve_result
                && !legacy_failure
            {
                let revision =
                    crate::download::current_download_revision(&transaction, &result.task_id)?;
                crate::download::persist_download_result(&transaction, result, revision, false)?;
            }
            forget_pending_cancelled_finish(&transaction, &result.task_id, &result.item_id)?;
            transaction.commit()?;
            return Ok(SchedulerCancelledFinish::Settled(None));
        }
        if legacy_failure
            && (runtime.task_paused || !runtime.active_item_ids.contains(&result.item_id))
        {
            return Err(StoreError::invalid(
                "legacy generation failure requires an unpaused active item",
            ));
        }
        reject_closed_task(&transaction, &result.task_id)?;
        let request = StoreUpdateTaskRequest {
            task_id: result.task_id.clone(),
            expected_revision: runtime.revision,
            command: StoreTaskCommand {
                kind: if legacy_failure {
                    TaskStateCommandKind::MarkFailed
                } else {
                    TaskStateCommandKind::CancelItems
                },
                item_ids: vec![result.item_id.clone()],
            },
            event_id: event_id.to_owned(),
            occurred_at: result.finished_at.clone(),
        };
        request.validate().map_err(StoreError::invalid)?;
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("event timestamp is required"));
        }
        let request_json = serde_json::to_string(&request).map_err(StoreError::Corrupt)?;
        let mut next = runtime.clone();
        let state = selection_state_from_runtime(&runtime)?;
        let command = store_command(&transaction, &result.task_id, &request.command)?;
        let reduced = reduce_task_state(&state, &command).map_err(|error| {
            StoreError::invalid(format!("{}: {}", error.code, error.user_message))
        })?;
        apply_selection_state(&mut next, reduced);
        next.revision = runtime
            .revision
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
        next.updated_at = result.finished_at.clone();
        next.status = lifecycle_status(&next);
        next.validate().map_err(StoreError::invalid)?;
        if !preserve_result && !legacy_failure {
            let revision =
                crate::download::current_download_revision(&transaction, &result.task_id)?;
            crate::download::persist_download_result(&transaction, result, revision, false)?;
        }
        let error_json = legacy_failure
            .then(scheduler_generation_uncertain_failure)
            .map(|failure| serde_json::to_string(&failure))
            .transpose()
            .map_err(StoreError::Corrupt)?;
        let runtime_json = serde_json::to_string(&next).map_err(StoreError::Corrupt)?;
        let changed = transaction.execute(
            "UPDATE task_runtime SET status=?1, revision=?2, updated_at=?3, state_json=?4 WHERE task_id=?5 AND revision=?6",
            params![
                lifecycle_status_name(&next.status),
                sqlite_integer(next.revision, "task revision")?,
                next.updated_at,
                runtime_json,
                next.task_id,
                sqlite_integer(runtime.revision, "expected task revision")?
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        transaction.execute(
            "INSERT INTO task_events(event_id,task_id,sequence,occurred_at,event_type,item_id,payload_json,error_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                request.event_id,
                request.task_id,
                sqlite_integer(next.revision, "task event sequence")?,
                request.occurred_at,
                command_name(&request.command.kind),
                result.item_id,
                request_json,
                error_json,
            ],
        )?;
        forget_pending_cancelled_finish(&transaction, &result.task_id, &result.item_id)?;
        transaction.commit()?;
        Ok(SchedulerCancelledFinish::Settled(Some(next)))
    }

    pub fn list_task_events(&self, task_id: &str) -> Result<Vec<StoredTaskEvent>, StoreError> {
        if !crate::core_restore::task_is_actionable(&self.connection, task_id)? {
            return Err(StoreError::NotFound);
        }
        let mut statement = self.connection.prepare(
            "SELECT event_id, task_id, sequence, occurred_at, event_type, item_id, \
                    payload_json, error_json \
             FROM task_events WHERE task_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (event_id, task_id, sequence, occurred_at, event_type, item_id, payload, error) =
                row?;
            let sequence = u64::try_from(sequence)
                .map_err(|_| StoreError::invalid("negative task event sequence"))?;
            Ok(StoredTaskEvent {
                event_id,
                task_id,
                sequence,
                occurred_at,
                event_type,
                item_id,
                payload: serde_json::from_str(&payload).map_err(StoreError::Corrupt)?,
                error: error
                    .map(|value| serde_json::from_str(&value).map_err(StoreError::Corrupt))
                    .transpose()?,
            })
        })
        .collect()
    }

    pub fn load_task_event_page(
        &self,
        request: &StoreTaskEventPageRequest,
    ) -> Result<StoreTaskEventPage, StoreError> {
        if !valid_bounded_scalar(&request.task_id, 128)
            || request.page_size == 0
            || request.page_size > MAX_TASK_EVENT_PAGE_SIZE
        {
            return Err(StoreError::invalid("task event page request is invalid"));
        }
        if !crate::core_restore::task_is_actionable(&self.connection, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        let tombstoned: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [&request.task_id],
            |row| row.get(0),
        )?;
        if tombstoned {
            return Err(StoreError::NotFound);
        }
        let revision: i64 = self
            .connection
            .query_row(
                "SELECT revision FROM task_runtime WHERE task_id = ?1",
                [&request.task_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let revision = u64::try_from(revision)
            .map_err(|_| StoreError::invalid("negative task event page revision"))?;
        // Event history is read-only. Observe the current revision in the
        // response instead of stale-failing while a live task advances.
        let before_sequence = request
            .before_sequence
            .map(|value| sqlite_integer(value, "task event cursor"))
            .transpose()?;
        let limit = request
            .page_size
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task event page limit overflow"))?;
        let limit = i64::from(limit);
        let mut statement = self.connection.prepare(
            "SELECT event.sequence, event.occurred_at, event.event_type, event.item_id,
                    event.error_json,
                    CASE WHEN event.event_type IN ('mark_failed', 'failed')
                              AND json_valid(result.result_json)
                         THEN json_extract(result.result_json, '$.failure.code') END,
                    CASE WHEN event.event_type = 'pause_task'
                              AND json_valid(event.payload_json)
                         THEN json_extract(event.payload_json, '$.pause_reason.kind') END
             FROM task_events event
             LEFT JOIN download_results result
               ON result.task_id = event.task_id AND result.item_id = event.item_id
             WHERE event.task_id = ?1 AND (?2 IS NULL OR event.sequence < ?2)
             ORDER BY event.sequence DESC LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![request.task_id, before_sequence, limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?;
        let mut items = Vec::new();
        for row in rows {
            let (sequence, occurred_at, event_type, item_id, error_json, result_code, pause_kind) =
                row?;
            let sequence = u64::try_from(sequence)
                .map_err(|_| StoreError::invalid("negative task event sequence"))?;
            if !valid_bounded_scalar(&occurred_at, 30) {
                return Err(StoreError::invalid("task event timestamp is invalid"));
            }
            let disk_low = pause_kind.as_deref() == Some("disk_low");
            let (level, event_kind, message_zh) = if disk_low {
                ("warning", "paused", "磁盘空间不足，任务已自动暂停")
            } else {
                public_event_description(&event_type)
            };
            let item_label = item_id.and_then(public_item_label);
            let code = error_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|value| {
                    value
                        .get("code")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .or(result_code)
                .or_else(|| {
                    disk_low.then(|| insar_core::storage_guard::DISK_LOW_ERROR_CODE.to_owned())
                })
                .filter(|value| valid_event_code(value));
            let message_zh = if event_type == "mark_failed"
                && code.as_deref() == Some(SCHEDULER_GENERATION_UNCERTAIN)
            {
                SCHEDULER_GENERATION_UNCERTAIN_MESSAGE
            } else {
                message_zh
            };
            items.push(StoreTaskEventPageItem {
                sequence,
                occurred_at,
                level: level.to_owned(),
                event_kind: event_kind.to_owned(),
                item_label,
                code,
                message_zh: message_zh.to_owned(),
            });
        }
        let has_more = items.len() > request.page_size as usize;
        items.truncate(request.page_size as usize);
        let next_before_sequence = has_more
            .then(|| items.last().map(|item| item.sequence))
            .flatten();
        Ok(StoreTaskEventPage {
            task_id: request.task_id.clone(),
            revision,
            page_size: request.page_size,
            has_more,
            next_before_sequence,
            items,
        })
    }
}

fn classify_cancelled_generation(
    connection: &Connection,
    runtime: &TaskRuntimeSnapshot,
    item_id: &str,
    started_revision: u64,
) -> Result<CancelledGeneration, StoreError> {
    const MAX_EVENTS: usize = 4096;
    let mut statement = connection.prepare(
        "SELECT event_type, payload_json, sequence, event_id, occurred_at \
         FROM task_events \
         WHERE task_id = ?1 AND sequence > ?2 AND sequence <= ?3 \
           AND event_type IN ('cancel_task','cancel_items','retry_task','retry_items') \
         ORDER BY sequence LIMIT ?4",
    )?;
    let mut rows = statement.query(params![
        runtime.task_id,
        sqlite_integer(started_revision, "worker start revision")?,
        sqlite_integer(runtime.revision, "current task revision")?,
        (MAX_EVENTS + 1) as i64,
    ])?;
    let mut seen = 0;
    let mut cancelled = false;
    let mut retried = false;
    let mut legacy_ambiguous = false;
    let mut legacy_cancel_pending = false;
    while let Some(row) = rows.next()? {
        seen += 1;
        if seen > MAX_EVENTS {
            return Err(StoreError::invalid(
                "cancel/retry event evidence exceeds scan limit",
            ));
        }
        let kind: String = row.get(0)?;
        let payload: String = row.get(1)?;
        let sequence: i64 = row.get(2)?;
        let stored_event_id: String = row.get(3)?;
        let occurred_at: String = row.get(4)?;
        let request: StoreUpdateTaskRequest =
            serde_json::from_str(&payload).map_err(StoreError::Corrupt)?;
        if request.task_id != runtime.task_id
            || request.event_id != stored_event_id
            || request.occurred_at != occurred_at
            || request.expected_revision.checked_add(1) != u64::try_from(sequence).ok()
        {
            return Err(StoreError::invalid(
                "task event identity or revision differs",
            ));
        }
        // Task-level persisted requests carry the effective membership, unlike
        // their external command. Validate the external shape without trusting
        // that stamped membership or interpreting an empty stamp as all items.
        let task_level = matches!(
            request.command.kind,
            TaskStateCommandKind::CancelTask | TaskStateCommandKind::RetryTask
        );
        let mut external = request.clone();
        if task_level {
            external.command.item_ids.clear();
        }
        external.validate().map_err(StoreError::invalid)?;
        if request.occurred_at.trim().is_empty()
            || request
                .command
                .item_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != request.command.item_ids.len()
            || request
                .command
                .item_ids
                .iter()
                .any(|id| !runtime.selected_item_ids.contains(id))
        {
            return Err(StoreError::invalid("task event membership is invalid"));
        }
        let is_cancel = match (kind.as_str(), request.command.kind) {
            ("cancel_task", TaskStateCommandKind::CancelTask)
            | ("cancel_items", TaskStateCommandKind::CancelItems) => true,
            ("retry_task", TaskStateCommandKind::RetryTask)
            | ("retry_items", TaskStateCommandKind::RetryItems) => false,
            _ => {
                return Err(StoreError::invalid(
                    "task event kind does not match its command",
                ))
            }
        };
        if request.command.item_ids.is_empty() {
            if !task_level {
                return Err(StoreError::invalid("item event lacks item-scoped evidence"));
            }
            // A lone or truncated empty task-level event is missing evidence,
            // not permission to write a generation-uncertain failure.
            if is_cancel == legacy_cancel_pending {
                return Err(StoreError::invalid(
                    "legacy cancel/retry pair is incomplete",
                ));
            }
            legacy_cancel_pending = is_cancel;
            legacy_ambiguous = true;
            // Only a complete explicit pair AFTER the last ambiguous event
            // proves a successor generation. No empty command supplies a member.
            cancelled = false;
            retried = false;
        } else if request.command.item_ids.iter().any(|id| id == item_id) {
            if is_cancel {
                cancelled = true;
            } else if cancelled {
                retried = true;
            }
        }
    }
    if legacy_cancel_pending {
        return Err(StoreError::invalid(
            "legacy cancel/retry pair is incomplete",
        ));
    }
    if retried {
        Ok(CancelledGeneration::Retried)
    } else if legacy_ambiguous {
        Ok(CancelledGeneration::LegacyAmbiguous)
    } else {
        Ok(CancelledGeneration::Current)
    }
}

fn task_event_request_matches_stamped_payload(
    stored_json: &str,
    request: &StoreUpdateTaskRequest,
) -> bool {
    let Ok(mut stored) = serde_json::from_str::<StoreUpdateTaskRequest>(stored_json) else {
        return false;
    };
    if stored.task_id != request.task_id
        || stored.expected_revision != request.expected_revision
        || stored.event_id != request.event_id
        || stored.occurred_at != request.occurred_at
        || stored.command.kind != request.command.kind
    {
        return false;
    }
    if !matches!(
        request.command.kind,
        TaskStateCommandKind::CancelTask | TaskStateCommandKind::RetryTask
    ) {
        return stored.command.item_ids == request.command.item_ids;
    }
    stored.command.item_ids.clear();
    stored.command.item_ids == request.command.item_ids
}

/// The command body of `update_task`, shared with transactions that first
/// change frozen item bindings (retry fallback) and must retry atomically.
pub(crate) fn update_task_in_transaction(
    transaction: &Transaction<'_>,
    request: &StoreUpdateTaskRequest,
    request_json: &str,
) -> Result<StoredTask, StoreError> {
    update_task_in_transaction_with_options(transaction, request, request_json, false)
}

fn update_task_in_transaction_with_options(
    transaction: &Transaction<'_>,
    request: &StoreUpdateTaskRequest,
    request_json: &str,
    allow_archived_item_retry: bool,
) -> Result<StoredTask, StoreError> {
    // A persistent reacquire identity remains reserved even if its event was pruned.
    // Legacy events without a receipt are never promoted solely by a replay.
    if request.command.kind == TaskStateCommandKind::ReacquireItems {
        match crate::source_receipts::reacquire_receipt_status(transaction, request)? {
            ReacquireEventStatus::Committed => {
                reject_tombstoned_task(transaction, &request.task_id)?;
                if !crate::core_restore::task_is_actionable(transaction, &request.task_id)? {
                    return Err(StoreError::NotFound);
                }
                return load_compact_task_connection(transaction, &request.task_id)?
                    .ok_or(StoreError::NotFound);
            }
            ReacquireEventStatus::Conflict => return Err(StoreError::AlreadyExists),
            ReacquireEventStatus::Unknown => {}
        }
    } else if crate::source_receipts::has_reacquire_receipt(transaction, &request.event_id)? {
        return Err(StoreError::AlreadyExists);
    }
    let existing = transaction
        .query_row(
            "SELECT task_id, payload_json, event_type, occurred_at, sequence FROM task_events WHERE event_id = ?1",
            [&request.event_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?)),
        )
        .optional()?;
    if let Some((task_id, payload_json, event_type, occurred_at, sequence)) = existing {
        if task_id != request.task_id {
            return Err(StoreError::AlreadyExists);
        }
        if payload_json != request_json
            && !task_event_request_matches_stamped_payload(&payload_json, request)
        {
            return Err(StoreError::AlreadyExists);
        }
        if request.command.kind == TaskStateCommandKind::ReacquireItems
            && (event_type != "reacquire_items"
                || occurred_at != request.occurred_at
                || u64::try_from(sequence).ok() != request.expected_revision.checked_add(1))
        {
            return Err(StoreError::AlreadyExists);
        }
        if matches!(
            request.command.kind,
            TaskStateCommandKind::RetryTask | TaskStateCommandKind::RetryItems
        ) && (request.command.kind == TaskStateCommandKind::RetryTask
            || allow_archived_item_retry)
            && transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_history WHERE task_id = ?1)",
                [&request.task_id],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(StoreError::invalid(
                "archived task must be retried with a new event",
            ));
        }
        if !crate::core_restore::task_is_actionable(transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        let tombstoned: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [&request.task_id],
            |row| row.get(0),
        )?;
        if tombstoned {
            return Err(StoreError::NotFound);
        }
        return load_compact_task_connection(transaction, &request.task_id)?
            .ok_or(StoreError::NotFound);
    }

    let archived_retry = if request.command.kind == TaskStateCommandKind::RetryTask
        || (allow_archived_item_retry && request.command.kind == TaskStateCommandKind::RetryItems)
    {
        transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_history WHERE task_id = ?1)",
            [&request.task_id],
            |row| row.get::<_, bool>(0),
        )?
    } else {
        false
    };
    if !archived_retry {
        reject_closed_task(transaction, &request.task_id)?;
    }
    if !crate::core_restore::task_is_actionable(transaction, &request.task_id)? {
        return Err(StoreError::NotFound);
    }
    let mut task = if matches!(
        request.command.kind,
        TaskStateCommandKind::RetryTask
            | TaskStateCommandKind::RetryItems
            | TaskStateCommandKind::ReacquireItems
    ) {
        load_compact_task_connection(transaction, &request.task_id)?
    } else {
        load_scheduler_task_connection(transaction, &request.task_id)?
    }
    .ok_or(StoreError::NotFound)?;
    if task.runtime.revision != request.expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    // A cancelled task in history is retried like a failed one (L4 r10
    // P1-1): its cancelled files are queued again on its own id.
    if archived_retry
        && !matches!(
            task.runtime.status,
            TaskLifecycleStatus::Failed | TaskLifecycleStatus::Cancelled
        )
    {
        return Err(StoreError::invalid(
            "only failed or cancelled history tasks can be retried",
        ));
    }
    if task.snapshot.operation.as_ref().is_some_and(|operation| {
        operation.kind == "gacos" && operation.approved_worker_operation == "gacos_manual_request"
    }) && matches!(
        request.command.kind,
        TaskStateCommandKind::ResumeTask
            | TaskStateCommandKind::ResumeItems
            | TaskStateCommandKind::RetryTask
            | TaskStateCommandKind::RetryItems
            | TaskStateCommandKind::ReacquireItems
            | TaskStateCommandKind::ActivateItems
            | TaskStateCommandKind::MarkCompleted
            | TaskStateCommandKind::MarkSkipped
    ) {
        return Err(StoreError::GacosManualActionRequired);
    }
    let command = store_command(transaction, &request.task_id, &request.command)?;
    let request_json = if matches!(
        request.command.kind,
        TaskStateCommandKind::CancelTask | TaskStateCommandKind::RetryTask
    ) {
        let mut stamped = request.clone();
        stamped.command.item_ids = match &command {
            TaskStateCommand::CancelItems { item_ids }
            | TaskStateCommand::RetryItems { item_ids } => item_ids.iter().cloned().collect(),
            _ => {
                return Err(StoreError::invalid(
                    "task-level command did not expand to items",
                ))
            }
        };
        serde_json::to_string(&stamped).map_err(StoreError::Corrupt)?
    } else {
        request_json.to_owned()
    };
    let state = selection_state(&task)?;
    let mut next = reduce_task_state(&state, &command)
        .map_err(|error| StoreError::invalid(format!("{}: {}", error.code, error.user_message)))?;
    // A task-level retry of a finished (failed or cancelled) task runs it
    // again: a pause left from before it finished (pause, then cancel) no
    // longer holds the retried files (L4 r10 P1-1).
    if matches!(
        request.command.kind,
        TaskStateCommandKind::RetryTask | TaskStateCommandKind::RetryItems
    ) && matches!(
        task.runtime.status,
        TaskLifecycleStatus::Failed | TaskLifecycleStatus::Cancelled
    ) {
        next.task_paused = false;
    }
    let revision = request
        .expected_revision
        .checked_add(1)
        .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
    apply_selection_state(&mut task.runtime, next);
    task.runtime.revision = revision;
    task.runtime.updated_at = request.occurred_at.clone();
    task.runtime.status = lifecycle_status(&task.runtime);
    task.runtime.validate().map_err(StoreError::invalid)?;

    let revision_sql = sqlite_integer(revision, "task revision")?;
    let expected_sql = sqlite_integer(request.expected_revision, "expected task revision")?;
    let runtime_json = serde_json::to_string(&task.runtime).map_err(StoreError::Corrupt)?;
    let changed = transaction.execute(
        "UPDATE task_runtime SET status = ?1, revision = ?2, updated_at = ?3, state_json = ?4 \
         WHERE task_id = ?5 AND revision = ?6",
        params![
            lifecycle_status_name(&task.runtime.status),
            revision_sql,
            task.runtime.updated_at,
            runtime_json,
            task.runtime.task_id,
            expected_sql
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::RevisionConflict);
    }
    transaction.execute(
        "INSERT INTO task_events( \
            event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json \
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            request.event_id,
            request.task_id,
            revision_sql,
            request.occurred_at,
            command_name(&request.command.kind),
            single_item_id(&request.command),
            request_json
        ],
    )?;
    if archived_retry {
        // Restore the exact frozen task and its original ID in this transaction;
        // a failed retry leaves one task in the queue until archived again.
        transaction.execute(
            "DELETE FROM task_history WHERE task_id = ?1",
            [&request.task_id],
        )?;
    }
    if request.command.kind == TaskStateCommandKind::ReacquireItems {
        // Downloader D3: the worker gate renames the reused file aside
        // (kept) and downloads; the decision rows say so in this transaction.
        crate::local_reuse::record_reacquire_requests(
            transaction,
            &request.task_id,
            &request.command.item_ids,
            &request.occurred_at,
        )?;
        crate::source_receipts::insert_reacquire_receipt(transaction, request, revision)?;
    }
    Ok(task)
}

pub(crate) fn append_task_membership_in_transaction(
    transaction: &Transaction<'_>,
    mut task: StoredTask,
    expected_revision: u64,
    item_ids: &[String],
    event_id: &str,
    occurred_at: &str,
    payload_json: &str,
) -> Result<StoredTask, StoreError> {
    if task.runtime.revision != expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    if !matches!(
        task.runtime.status,
        TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running | TaskLifecycleStatus::Paused
    ) {
        return Err(StoreError::invalid(
            "new products may only be appended to queued, running, or paused tasks",
        ));
    }
    let appended: BTreeSet<&str> = item_ids.iter().map(String::as_str).collect();
    if item_ids.is_empty() || appended.len() != item_ids.len() {
        return Err(StoreError::invalid(
            "append item IDs must be a non-empty unique set",
        ));
    }
    let candidates: BTreeSet<&str> = task
        .snapshot
        .candidates
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    let selected: BTreeSet<&str> = task
        .runtime
        .selected_item_ids
        .iter()
        .map(String::as_str)
        .collect();
    if !appended.is_subset(&candidates) || !appended.is_disjoint(&selected) {
        return Err(StoreError::invalid(
            "appended items must be unselected products from the task source batch",
        ));
    }
    let frozen_products = item_ids
        .iter()
        .map(|item_id| {
            let item = task
                .snapshot
                .candidates
                .iter()
                .find(|candidate| candidate.item_id == *item_id)
                .ok_or_else(|| StoreError::invalid("appended product is outside source batch"))?;
            serde_json::to_string(item).map_err(StoreError::Corrupt)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let revision = expected_revision
        .checked_add(1)
        .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
    let revision_sql = sqlite_integer(revision, "task revision")?;
    for (position, (item_id, item_json)) in item_ids.iter().zip(frozen_products).enumerate() {
        transaction.execute(
            "INSERT INTO task_membership_additions(
                 task_id, item_id, added_revision, position, added_at, item_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                task.runtime.task_id,
                item_id,
                revision_sql,
                i64::try_from(position)
                    .map_err(|_| StoreError::invalid("append position exceeds SQLite range"))?,
                occurred_at,
                item_json,
            ],
        )?;
    }

    task.runtime
        .selected_item_ids
        .extend(item_ids.iter().cloned());
    task.runtime
        .queued_item_ids
        .extend(item_ids.iter().cloned());
    task.runtime.revision = revision;
    task.runtime.updated_at = occurred_at.to_owned();
    task.runtime.status = lifecycle_status(&task.runtime);
    task.runtime.validate().map_err(StoreError::invalid)?;
    let runtime_json = serde_json::to_string(&task.runtime).map_err(StoreError::Corrupt)?;
    let changed = transaction.execute(
        "UPDATE task_runtime SET status = ?1, revision = ?2, updated_at = ?3, state_json = ?4
         WHERE task_id = ?5 AND revision = ?6",
        params![
            lifecycle_status_name(&task.runtime.status),
            revision_sql,
            task.runtime.updated_at,
            runtime_json,
            task.runtime.task_id,
            sqlite_integer(expected_revision, "expected task revision")?,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::RevisionConflict);
    }
    transaction.execute(
        "INSERT INTO task_events(
             event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json
         ) VALUES (?1, ?2, ?3, ?4, 'append_items', NULL, ?5, NULL)",
        params![
            event_id,
            task.runtime.task_id,
            revision_sql,
            occurred_at,
            payload_json,
        ],
    )?;
    let explicit =
        load_explicit_task_source_item_ids(transaction, &task.runtime.task_id)?.is_some();
    crate::source_index::index_task_items(transaction, &task.snapshot, explicit, item_ids)?;
    Ok(task)
}

/// Whether a migration rejection could retain the download rows it replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RejectionEvidence {
    /// The replaced result and progress rows are first retained as immutable
    /// refusal evidence, then the visible failure replaces them.
    Retained,
    /// That evidence could not be written, so no download row is replaced.
    Unretained,
}

fn validate_budget_upgrade_rejection(
    request: &StoreUpdateTaskRequest,
    result: &insar_core::DownloadItemResult,
) -> Result<(), StoreError> {
    result.validate().map_err(StoreError::invalid)?;
    let Some(failure) = result.failure.as_ref() else {
        return Err(StoreError::invalid(
            "budget migration rejection requires a local failure",
        ));
    };
    failure.validate().map_err(StoreError::invalid)?;
    if request.command.kind != TaskStateCommandKind::MarkFailed
        || request.command.item_ids.as_slice() != std::slice::from_ref(&result.item_id)
        || request.task_id != result.task_id
        || result.outcome != insar_core::DownloadOutcome::Failed
        || failure.code.len() > 512
        || failure.reason_zh.len() > 64 * 1024
    {
        return Err(StoreError::invalid(
            "invalid bounded budget migration rejection",
        ));
    }
    Ok(())
}

fn valid_bounded_scalar(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_event_code(value: &str) -> bool {
    valid_bounded_scalar(value, 64)
        && value.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn public_item_label(value: String) -> Option<String> {
    if !valid_bounded_scalar(&value, 256) {
        return None;
    }
    let label = value
        .rsplit_once("::ITEM::")
        .map(|(_, item)| item)
        .unwrap_or(value.as_str());
    valid_bounded_scalar(label, 256).then(|| label.to_owned())
}

fn public_event_description(event_type: &str) -> (&'static str, &'static str, &'static str) {
    match event_type {
        "task_created" => ("info", "started", "任务已创建"),
        "append_items" => ("info", "progress", "已添加下载文件"),
        "activate_items" | "mark_active" | "started" | "body_started" => {
            ("info", "started", "文件开始下载")
        }
        "progress" => ("info", "progress", "正在传输数据"),
        "pause_task" => ("warning", "paused", "任务已暂停"),
        "pause" => ("warning", "paused", "下载已暂停"),
        "pause_items" | "paused" | "body_paused" => ("warning", "paused", "文件已暂停"),
        "resume_task" => ("info", "resumed", "任务已继续"),
        "resume" => ("info", "resumed", "下载已继续"),
        "resume_items" | "resumed" => ("info", "resumed", "文件已继续"),
        "mark_completed" | "completed" => ("success", "completed", "文件下载完成"),
        "mark_skipped" => ("success", "completed", "已复用已有文件，判定依据见任务明细"),
        "mark_failed" | "failed" => ("error", "failed", "文件下载失败"),
        "cancel_task" => ("warning", "cancelled", "任务已结束"),
        "cancel_items" | "mark_cancelled" | "cancelled" => {
            ("warning", "cancelled", "文件下载已取消")
        }
        "retry_task" => ("info", "retried", "任务已重试"),
        "retry_items" | "retry" | "retried" => ("info", "retried", "文件已重试"),
        "reacquire_items" => ("info", "retried", "已请求重新下载，原文件将另存保留"),
        "local_reuse_applied" => (
            "success",
            "completed",
            "已复用已有文件（下载前扫描），判定依据见任务明细",
        ),
        "nisar_budget_upgrade" => ("info", "updated", "已按记账安全升级旧任务的请求预算"),
        "nisar_workspace_rebinding" => (
            "info",
            "updated",
            "已核对工作区目录身份，沿用改名前记录的路径继续",
        ),
        // Downloader enhancement D1b: why a NISAR writer has waited for its
        // output longer than 5 minutes.
        "nisar_output_wait_same_artifact" => (
            "warning",
            "updated",
            "写出已等待超过 5 分钟：本程序中另一项正在写出同一成果",
        ),
        "nisar_output_wait_other_process" => (
            "warning",
            "updated",
            "写出已等待超过 5 分钟：另一个程序正在写出同一成果",
        ),
        "nisar_output_wait_earlier_build" => (
            "warning",
            "updated",
            "写出已等待超过 5 分钟：旧版本程序正在向同一输出目录写出",
        ),
        "nisar_output_wait_output_slot" => (
            "warning",
            "updated",
            "写出已等待超过 5 分钟：其他条目正在占用写出槽",
        ),
        _ => ("info", "updated", "任务状态已更新"),
    }
}

pub(crate) fn reject_tombstoned_task(
    connection: &Connection,
    task_id: &str,
) -> Result<(), StoreError> {
    let deleted: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
        [task_id],
        |row| row.get(0),
    )?;
    if deleted {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

pub(crate) fn reject_closed_task(connection: &Connection, task_id: &str) -> Result<(), StoreError> {
    let closed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_history WHERE task_id = ?1 \
         UNION SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
        [task_id],
        |row| row.get(0),
    )?;
    if closed {
        Err(StoreError::invalid(
            "archived or deleted tasks are immutable",
        ))
    } else {
        Ok(())
    }
}

fn load_runtime_snapshot_connection(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<TaskRuntimeSnapshot>, StoreError> {
    let json = connection
        .query_row(
            "SELECT state_json FROM task_runtime WHERE task_id = ?1",
            [task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    json.map(|value| {
        let runtime: TaskRuntimeSnapshot =
            serde_json::from_str(&value).map_err(StoreError::Corrupt)?;
        if runtime.task_id != task_id {
            return Err(StoreError::invalid("runtime task ID differs"));
        }
        runtime.validate().map_err(StoreError::invalid)?;
        Ok(runtime)
    })
    .transpose()
}

fn reject_gacos_manual_scheduler_command(
    connection: &Connection,
    task_id: &str,
    kind: &TaskStateCommandKind,
) -> Result<(), StoreError> {
    if !matches!(
        kind,
        TaskStateCommandKind::ActivateItems
            | TaskStateCommandKind::MarkCompleted
            | TaskStateCommandKind::MarkSkipped
    ) {
        return Ok(());
    }
    let manual: Option<i64> = connection
        .query_row(
            "SELECT CASE WHEN json_valid(snapshot_json) \
                    AND json_extract(snapshot_json, '$.operation.kind') = 'gacos' \
                    AND json_extract(snapshot_json, '$.operation.approved_worker_operation') = 'gacos_manual_request' \
                  THEN 1 ELSE 0 END \
             FROM task_snapshots WHERE task_id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .optional()?;
    match manual {
        Some(1) => Err(StoreError::GacosManualActionRequired),
        Some(_) => Ok(()),
        None => Err(StoreError::NotFound),
    }
}

fn store_command(
    connection: &Connection,
    task_id: &str,
    command: &StoreTaskCommand,
) -> Result<TaskStateCommand, StoreError> {
    let item_ids: BTreeSet<String> = command.item_ids.iter().cloned().collect();
    if item_ids.len() != command.item_ids.len() {
        return Err(StoreError::invalid("task command item IDs must be unique"));
    }
    match command.kind {
        TaskStateCommandKind::PauseTask => no_items(command, TaskStateCommand::PauseTask),
        TaskStateCommandKind::ResumeTask => no_items(command, TaskStateCommand::ResumeTask),
        TaskStateCommandKind::RetryTask => {
            if !item_ids.is_empty() {
                return Err(StoreError::invalid(
                    "task-level retry does not accept item IDs",
                ));
            }
            let task =
                load_compact_task_connection(connection, task_id)?.ok_or(StoreError::NotFound)?;
            let retry_ids = task
                .runtime
                .failed_item_ids
                .iter()
                .chain(task.runtime.cancelled_item_ids.iter())
                .cloned()
                .collect();
            some_items(retry_ids, |item_ids| TaskStateCommand::RetryItems {
                item_ids,
            })
        }
        TaskStateCommandKind::CancelTask => {
            if !item_ids.is_empty() {
                return Err(StoreError::invalid(
                    "task-level cancel does not accept item IDs",
                ));
            }
            let task =
                load_scheduler_task_connection(connection, task_id)?.ok_or(StoreError::NotFound)?;
            let cancel_ids = task
                .runtime
                .active_item_ids
                .iter()
                .chain(task.runtime.paused_item_ids.iter())
                .chain(task.runtime.queued_item_ids.iter())
                .cloned()
                .collect();
            some_items(cancel_ids, |item_ids| TaskStateCommand::CancelItems {
                item_ids,
            })
        }
        TaskStateCommandKind::ActivateItems => some_items(item_ids, |item_ids| {
            TaskStateCommand::ActivateItems { item_ids }
        }),
        TaskStateCommandKind::PauseItems => some_items(item_ids, |item_ids| {
            TaskStateCommand::PauseItems { item_ids }
        }),
        TaskStateCommandKind::ResumeItems => some_items(item_ids, |item_ids| {
            TaskStateCommand::ResumeItems { item_ids }
        }),
        TaskStateCommandKind::RetryItems => some_items(item_ids, |item_ids| {
            TaskStateCommand::RetryItems { item_ids }
        }),
        TaskStateCommandKind::CancelItems => some_items(item_ids, |item_ids| {
            TaskStateCommand::CancelItems { item_ids }
        }),
        TaskStateCommandKind::MarkCompleted => one_item(item_ids, |item_id| {
            TaskStateCommand::MarkCompleted { item_id }
        }),
        TaskStateCommandKind::MarkFailed => {
            one_item(item_ids, |item_id| TaskStateCommand::MarkFailed { item_id })
        }
        TaskStateCommandKind::MarkSkipped => one_item(item_ids, |item_id| {
            TaskStateCommand::MarkSkipped { item_id }
        }),
        TaskStateCommandKind::ReacquireItems => some_items(item_ids, |item_ids| {
            TaskStateCommand::ReacquireItems { item_ids }
        }),
    }
}

fn no_items(
    command: &StoreTaskCommand,
    result: TaskStateCommand,
) -> Result<TaskStateCommand, StoreError> {
    if command.item_ids.is_empty() {
        Ok(result)
    } else {
        Err(StoreError::invalid("task command does not accept item IDs"))
    }
}

fn some_items(
    item_ids: BTreeSet<String>,
    build: impl FnOnce(BTreeSet<String>) -> TaskStateCommand,
) -> Result<TaskStateCommand, StoreError> {
    if item_ids.is_empty() {
        Err(StoreError::invalid("task command requires item IDs"))
    } else {
        Ok(build(item_ids))
    }
}

fn one_item(
    item_ids: BTreeSet<String>,
    build: impl FnOnce(String) -> TaskStateCommand,
) -> Result<TaskStateCommand, StoreError> {
    if item_ids.len() != 1 {
        return Err(StoreError::invalid(
            "task command requires exactly one item ID",
        ));
    }
    Ok(build(
        item_ids.into_iter().next().expect("one item was checked"),
    ))
}

impl TaskStore {
    /// Downloader D3-2: commits items the local pre-scan proved reusable in
    /// one IMMEDIATE transaction: the queued items still in the queue become
    /// skipped with one task revision and one `local_reuse_applied` event
    /// (counts only, no paths), each with its skipped result (attempt 0,
    /// bytes = file size), decision row and optional ledger row. A paused or
    /// closed task, and items that left the queue, take nothing. Returns the
    /// applied item IDs.
    pub fn apply_local_reuse_batch(
        &mut self,
        batch: &crate::StoreLocalReuseBatch,
    ) -> Result<Vec<String>, StoreError> {
        crate::local_reuse::validate_local_reuse_batch(batch)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        reject_closed_task(&transaction, &batch.task_id)?;
        let mut task = load_scheduler_task_connection(&transaction, &batch.task_id)?
            .ok_or(StoreError::NotFound)?;
        if task.runtime.task_paused {
            return Ok(Vec::new());
        }
        let queued: BTreeSet<&str> = task
            .runtime
            .queued_item_ids
            .iter()
            .map(String::as_str)
            .collect();
        let items: Vec<&crate::StoreLocalReuseBatchItem> = batch
            .items
            .iter()
            .filter(|item| queued.contains(item.item_id.as_str()))
            .collect();
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let mut state = selection_state(&task)?;
        let mut grades = serde_json::Map::new();
        for item in &items {
            state = reduce_task_state(
                &state,
                &TaskStateCommand::MarkSkipped {
                    item_id: item.item_id.clone(),
                },
            )
            .map_err(|error| {
                StoreError::invalid(format!("{}: {}", error.code, error.user_message))
            })?;
            let count = grades.get(&item.grade).and_then(Value::as_u64).unwrap_or(0);
            grades.insert(item.grade.clone(), Value::from(count + 1));
        }
        let expected = task.runtime.revision;
        let revision = expected
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task revision overflow"))?;
        apply_selection_state(&mut task.runtime, state);
        task.runtime.revision = revision;
        task.runtime.updated_at = batch.occurred_at.clone();
        task.runtime.status = lifecycle_status(&task.runtime);
        task.runtime.validate().map_err(StoreError::invalid)?;
        let runtime_json = serde_json::to_string(&task.runtime).map_err(StoreError::Corrupt)?;
        let revision_sql = sqlite_integer(revision, "task revision")?;
        let changed = transaction.execute(
            "UPDATE task_runtime SET status = ?1, revision = ?2, updated_at = ?3, state_json = ?4 \
             WHERE task_id = ?5 AND revision = ?6",
            params![
                lifecycle_status_name(&task.runtime.status),
                revision_sql,
                task.runtime.updated_at,
                runtime_json,
                task.runtime.task_id,
                sqlite_integer(expected, "expected task revision")?
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        let payload = serde_json::json!({ "items": items.len(), "grades": grades });
        transaction.execute(
            "INSERT INTO task_events( \
                event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json \
             ) VALUES (?1, ?2, ?3, ?4, 'local_reuse_applied', NULL, ?5, NULL)",
            params![
                batch.event_id,
                batch.task_id,
                revision_sql,
                batch.occurred_at,
                payload.to_string()
            ],
        )?;
        let mut applied = Vec::with_capacity(items.len());
        for item in items {
            let result = insar_core::DownloadItemResult {
                task_id: batch.task_id.clone(),
                item_id: item.item_id.clone(),
                outcome: insar_core::DownloadOutcome::Skipped,
                destination: item.destination.clone(),
                bytes_written: item.size_bytes,
                attempt_count: 0,
                failure: None,
                finished_at: batch.occurred_at.clone(),
            };
            let download_revision =
                crate::download::current_download_revision(&transaction, &batch.task_id)?;
            crate::download::persist_download_result(
                &transaction,
                &result,
                download_revision,
                false,
            )?;
            crate::local_reuse::upsert_task_item_local_reuse(
                &transaction,
                &crate::StoreTaskItemLocalReuse {
                    task_id: batch.task_id.clone(),
                    item_id: item.item_id.clone(),
                    disposition: crate::StoreLocalReuseDisposition::Reused {
                        grade: item.grade.clone(),
                    },
                    size_bytes: Some(item.size_bytes),
                    decided_at: batch.occurred_at.clone(),
                },
            )?;
            if let Some(landed) = &item.landed {
                crate::local_reuse::upsert_landed_artifact(&transaction, landed)?;
            }
            applied.push(item.item_id.clone());
        }
        transaction.commit()?;
        Ok(applied)
    }
}

fn selection_state(task: &StoredTask) -> Result<TaskSelectionState, StoreError> {
    let concurrency = usize::try_from(task.runtime.concurrency)
        .map_err(|_| StoreError::invalid("task concurrency exceeds platform range"))?;
    Ok(TaskSelectionState {
        task_id: task.runtime.task_id.clone(),
        candidate_ids: task
            .snapshot
            .candidates
            .iter()
            .map(|item| item.item_id.clone())
            .collect(),
        selected_ids: task.runtime.selected_item_ids.iter().cloned().collect(),
        active_ids: task.runtime.active_item_ids.iter().cloned().collect(),
        paused_ids: task.runtime.paused_item_ids.iter().cloned().collect(),
        queued_ids: task.runtime.queued_item_ids.iter().cloned().collect(),
        completed_ids: task.runtime.completed_item_ids.iter().cloned().collect(),
        failed_ids: task.runtime.failed_item_ids.iter().cloned().collect(),
        skipped_ids: task.runtime.skipped_item_ids.iter().cloned().collect(),
        cancelled_ids: task.runtime.cancelled_item_ids.iter().cloned().collect(),
        task_paused: task.runtime.task_paused,
        concurrency,
    })
}

fn selection_state_from_runtime(
    runtime: &TaskRuntimeSnapshot,
) -> Result<TaskSelectionState, StoreError> {
    let concurrency = usize::try_from(runtime.concurrency)
        .map_err(|_| StoreError::invalid("task concurrency exceeds platform range"))?;
    Ok(TaskSelectionState {
        task_id: runtime.task_id.clone(),
        candidate_ids: runtime.selected_item_ids.iter().cloned().collect(),
        selected_ids: runtime.selected_item_ids.iter().cloned().collect(),
        active_ids: runtime.active_item_ids.iter().cloned().collect(),
        paused_ids: runtime.paused_item_ids.iter().cloned().collect(),
        queued_ids: runtime.queued_item_ids.iter().cloned().collect(),
        completed_ids: runtime.completed_item_ids.iter().cloned().collect(),
        failed_ids: runtime.failed_item_ids.iter().cloned().collect(),
        skipped_ids: runtime.skipped_item_ids.iter().cloned().collect(),
        cancelled_ids: runtime.cancelled_item_ids.iter().cloned().collect(),
        task_paused: runtime.task_paused,
        concurrency,
    })
}

fn apply_selection_state(runtime: &mut TaskRuntimeSnapshot, state: TaskSelectionState) {
    runtime.active_item_ids = state.active_ids.into_iter().collect();
    runtime.paused_item_ids = state.paused_ids.into_iter().collect();
    runtime.queued_item_ids = state.queued_ids.into_iter().collect();
    runtime.completed_item_ids = state.completed_ids.into_iter().collect();
    runtime.failed_item_ids = state.failed_ids.into_iter().collect();
    runtime.skipped_item_ids = state.skipped_ids.into_iter().collect();
    runtime.cancelled_item_ids = state.cancelled_ids.into_iter().collect();
    runtime.task_paused = state.task_paused;
}

pub(crate) fn lifecycle_status(runtime: &TaskRuntimeSnapshot) -> TaskLifecycleStatus {
    let terminal_count = runtime.completed_item_ids.len()
        + runtime.failed_item_ids.len()
        + runtime.skipped_item_ids.len()
        + runtime.cancelled_item_ids.len();
    if terminal_count == runtime.selected_item_ids.len() {
        if !runtime.failed_item_ids.is_empty() {
            TaskLifecycleStatus::Failed
        } else if !runtime.cancelled_item_ids.is_empty() {
            TaskLifecycleStatus::Cancelled
        } else {
            TaskLifecycleStatus::Completed
        }
    } else if runtime.task_paused
        || (runtime.active_item_ids.is_empty()
            && runtime.queued_item_ids.is_empty()
            && !runtime.paused_item_ids.is_empty())
    {
        // Paused items alone also pause the task: nothing can run until the user resumes, and a
        // queued label would hide the task card's resume control.
        TaskLifecycleStatus::Paused
    } else if runtime.active_item_ids.is_empty() {
        TaskLifecycleStatus::Queued
    } else {
        TaskLifecycleStatus::Running
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

fn command_name(kind: &TaskStateCommandKind) -> &'static str {
    match kind {
        TaskStateCommandKind::PauseTask => "pause_task",
        TaskStateCommandKind::ResumeTask => "resume_task",
        TaskStateCommandKind::RetryTask => "retry_task",
        TaskStateCommandKind::CancelTask => "cancel_task",
        TaskStateCommandKind::ActivateItems => "activate_items",
        TaskStateCommandKind::PauseItems => "pause_items",
        TaskStateCommandKind::ResumeItems => "resume_items",
        TaskStateCommandKind::RetryItems => "retry_items",
        TaskStateCommandKind::MarkCompleted => "mark_completed",
        TaskStateCommandKind::MarkFailed => "mark_failed",
        TaskStateCommandKind::MarkSkipped => "mark_skipped",
        TaskStateCommandKind::CancelItems => "cancel_items",
        TaskStateCommandKind::ReacquireItems => "reacquire_items",
    }
}

fn single_item_id(command: &StoreTaskCommand) -> Option<&str> {
    (command.item_ids.len() == 1).then(|| command.item_ids[0].as_str())
}
