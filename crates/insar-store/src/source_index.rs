//! Schema-33 indexes over immutable search batches and task snapshots, and
//! the persisted per-source result views (C1 SourceView).
//!
//! Every table here is additive: search batches and task snapshots stay the
//! immutable authority, and each index can be rebuilt from them by
//! [`backfill_schema_v33`]. Batch publication and task creation write their
//! index rows in the same Store transaction.

use std::collections::HashSet;

use insar_core::{FrozenSearchBatch, FrozenTaskSnapshot, TypedError};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{StoreError, TaskStore};

pub(crate) const SCHEMA_V33_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS search_batch_headers (
    batch_id TEXT PRIMARY KEY NOT NULL REFERENCES search_batches(batch_id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    logical_dataset TEXT,
    native_collection_id TEXT,
    dataset TEXT,
    candidate_count INTEGER NOT NULL CHECK(candidate_count >= 0),
    -- 1 only for a user search publication (it activated the legacy Workbench
    -- slot). Derived deliveries and retry-fallback re-searches freeze batches
    -- with the same route keys but must never replace a source view.
    view_eligible INTEGER NOT NULL CHECK(view_eligible IN (0, 1))
) STRICT;
CREATE INDEX IF NOT EXISTS search_batch_headers_route_idx
    ON search_batch_headers(view_eligible, provider_id, logical_dataset, native_collection_id,
                            dataset, created_at DESC, batch_id DESC);

-- Outcome of the schema-33 index backfill: tasks whose snapshot the v2
-- reader could not decode were left unindexed (counted, never guessed).
CREATE TABLE IF NOT EXISTS source_index_backfill (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
    skipped_task_count INTEGER NOT NULL CHECK(skipped_task_count >= 0),
    recorded_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS source_views (
    resource_id TEXT PRIMARY KEY NOT NULL CHECK(length(resource_id) BETWEEN 1 AND 128),
    batch_id TEXT REFERENCES search_batches(batch_id) ON DELETE RESTRICT,
    item_count INTEGER NOT NULL CHECK(item_count >= 0),
    searched_at TEXT,
    pending_request_id TEXT CHECK(pending_request_id IS NULL OR length(pending_request_id) BETWEEN 1 AND 128),
    last_error_json TEXT CHECK(last_error_json IS NULL OR json_valid(last_error_json)),
    updated_at TEXT NOT NULL,
    CHECK((batch_id IS NULL) = (searched_at IS NULL))
) STRICT;
CREATE INDEX IF NOT EXISTS source_views_batch_idx ON source_views(batch_id);

CREATE TABLE IF NOT EXISTS batch_task_items (
    source_batch_id TEXT NOT NULL,
    source_item_id TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    task_item_id TEXT NOT NULL,
    relation TEXT NOT NULL CHECK(relation IN ('direct','explicit','derived')),
    PRIMARY KEY(source_batch_id, source_item_id, task_id, task_item_id)
) STRICT;
CREATE INDEX IF NOT EXISTS batch_task_items_task_idx ON batch_task_items(task_id, task_item_id);

CREATE TABLE IF NOT EXISTS task_item_rebinds (
    task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    snapshot_revision INTEGER NOT NULL CHECK(snapshot_revision >= 1),
    item_id TEXT NOT NULL,
    source_batch_id TEXT NOT NULL REFERENCES search_batches(batch_id) ON DELETE RESTRICT,
    item_json TEXT NOT NULL CHECK(json_valid(item_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY(task_id, snapshot_revision, item_id)
) STRICT;
CREATE TRIGGER IF NOT EXISTS task_item_rebinds_are_frozen
BEFORE UPDATE ON task_item_rebinds
BEGIN
    SELECT RAISE(ABORT, 'frozen task item rebind cannot be updated');
END;

-- Current snapshot revision of a task (0 = original frozen snapshot) and the
-- retry-fallback batch most recently frozen for it. A rebuild must name that
-- exact batch, frozen for the same failed task revision.
CREATE TABLE IF NOT EXISTS task_snapshot_heads (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE,
    snapshot_revision INTEGER NOT NULL CHECK(snapshot_revision >= 0),
    source_batch_id TEXT NOT NULL REFERENCES search_batches(batch_id) ON DELETE RESTRICT,
    offered_batch_id TEXT REFERENCES search_batches(batch_id) ON DELETE RESTRICT,
    offered_task_revision INTEGER CHECK(offered_task_revision IS NULL OR offered_task_revision >= 0),
    updated_at TEXT NOT NULL,
    CHECK((offered_batch_id IS NULL) = (offered_task_revision IS NULL))
) STRICT;
"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreBatchRouteHeader {
    pub batch_id: String,
    pub provider_id: String,
    pub created_at: String,
    pub logical_dataset: Option<String>,
    pub native_collection_id: Option<String>,
    pub dataset: Option<String>,
    pub item_count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSourceView {
    pub resource_id: String,
    pub batch_id: Option<String>,
    pub item_count: u64,
    pub searched_at: Option<String>,
    pub pending_request_id: Option<String>,
    pub last_error: Option<TypedError>,
    pub updated_at: String,
}

fn query_text<'a>(query: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    query.get(key).and_then(Value::as_str)
}

/// Header row for a batch inserted in the same transaction. `search_publication`
/// is true only for a user search that activates the Workbench slot.
pub(crate) fn insert_batch_header(
    connection: &Connection,
    batch: &FrozenSearchBatch,
    search_publication: bool,
) -> Result<(), StoreError> {
    insert_batch_header_with_count(
        connection,
        batch,
        batch.candidates.len() as u64,
        search_publication,
    )
}

/// #22 B slice 3(b) review fix 5: the streaming seal owns only an empty
/// header plus the exact candidate count it streamed, so the count is
/// supplied here instead of reading `batch.candidates.len()`.
pub(crate) fn insert_batch_header_with_count(
    connection: &Connection,
    batch: &FrozenSearchBatch,
    candidate_count: u64,
    search_publication: bool,
) -> Result<(), StoreError> {
    let view_eligible =
        search_publication && query_text(&batch.query, "derived_from_batch_id").is_none();
    connection.execute(
        "INSERT OR IGNORE INTO search_batch_headers(
             batch_id, provider_id, created_at, logical_dataset, native_collection_id, dataset,
             candidate_count, view_eligible
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            batch.batch_id,
            batch.provider_id,
            batch.created_at,
            query_text(&batch.query, "logical_dataset"),
            query_text(&batch.query, "native_collection_id"),
            query_text(&batch.query, "dataset"),
            i64::try_from(candidate_count)
                .map_err(|_| StoreError::invalid("candidate count exceeds SQLite range"))?,
            view_eligible,
        ],
    )?;
    Ok(())
}

/// Reverse links from source products to task items. `item_ids` limits the
/// rows to the selected (or newly appended) task items.
pub(crate) fn index_task_items(
    connection: &Connection,
    snapshot: &FrozenTaskSnapshot,
    explicit: bool,
    item_ids: &[String],
) -> Result<(), StoreError> {
    let wanted = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
    let derived_from = query_text(&snapshot.query, "derived_from_batch_id")
        .filter(|batch_id| *batch_id != snapshot.batch_id);
    // A progressive task can gain items from its original batch after the
    // small admission batch was frozen. Do not index those later products as
    // members of the admission batch. Use the same rule during backfill.
    let admission_ids = if crate::loading_admission::is_progressive_admission_query(&snapshot.query)
    {
        let count: Option<i64> = connection.query_row(
            "SELECT CASE WHEN json_valid(snapshot_json) THEN json_array_length(snapshot_json, '$.candidates') END \
             FROM search_batches WHERE batch_id = ?1",
            [&snapshot.batch_id],
            |row| row.get(0),
        ).optional()?.flatten();
        if count.is_none_or(|count| count <= 0 || count > crate::MAX_WORKBENCH_PAGE_SIZE as i64) {
            return Err(StoreError::invalid(
                "progressive admission batch count is invalid",
            ));
        }
        let batch = crate::repository::load_batch(connection, &snapshot.batch_id)?
            .ok_or(StoreError::NotFound)?;
        Some(
            batch
                .candidates
                .into_iter()
                .map(|item| item.item_id)
                .collect::<HashSet<_>>(),
        )
    } else {
        None
    };
    let mut insert = connection.prepare_cached(
        "INSERT OR IGNORE INTO batch_task_items(
             source_batch_id, source_item_id, task_id, task_item_id, relation
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for item in snapshot
        .candidates
        .iter()
        .filter(|item| wanted.contains(item.item_id.as_str()))
    {
        let backlink = item.metadata.get("source_item_id").and_then(Value::as_str);
        let direct = if explicit {
            backlink
        } else {
            Some(item.item_id.as_str())
        };
        if let Some(source) = direct.filter(|source| {
            admission_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(*source))
        }) {
            insert.execute(params![
                snapshot.batch_id,
                source,
                snapshot.task_id,
                item.item_id,
                if explicit { "explicit" } else { "direct" },
            ])?;
        }
        if let (Some(batch_id), Some(source)) = (derived_from, backlink) {
            insert.execute(params![
                batch_id,
                source,
                snapshot.task_id,
                item.item_id,
                "derived"
            ])?;
        }
    }
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )?)
}

/// A task row the v2 reader cannot decode: its snapshot or runtime content is
/// invalid, not the database. Every other error aborts the migration.
fn undecodable(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::Corrupt(_)
            | StoreError::InvalidSnapshot { .. }
            | StoreError::InvalidV3Snapshot { .. }
            | StoreError::SnapshotRowEnvelopeMismatch { .. }
            | StoreError::V3SnapshotRejectedByV2Reader
            | StoreError::UnsupportedSnapshotVersion
    )
}

/// Rebuild the derived indexes from immutable rows. Safe to repeat: every
/// insert is idempotent, and tasks whose snapshot the v2 reader cannot decode
/// are left unindexed instead of guessed. SQLite errors are returned so the
/// migration transaction rolls back. Returns the number of skipped tasks,
/// which is also recorded in `source_index_backfill`.
pub(crate) fn backfill_schema_v33(connection: &Connection) -> Result<u64, StoreError> {
    let skipped = backfill_schema_v33_indexes(connection)?;
    connection.execute(
        "INSERT INTO source_index_backfill(singleton, skipped_task_count, recorded_at)
         VALUES (1, ?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(singleton) DO UPDATE SET
             skipped_task_count = excluded.skipped_task_count,
             recorded_at = excluded.recorded_at",
        [i64::try_from(skipped).unwrap_or(i64::MAX)],
    )?;
    Ok(skipped)
}

fn backfill_schema_v33_indexes(connection: &Connection) -> Result<u64, StoreError> {
    if table_exists(connection, "search_batches")? {
        connection.execute_batch(
            "INSERT OR IGNORE INTO search_batch_headers(
                 batch_id, provider_id, created_at, logical_dataset, native_collection_id,
                 dataset, candidate_count, view_eligible
             )
             SELECT batch_id, provider_id, created_at,
                    CASE WHEN json_type(snapshot_json, '$.query.logical_dataset') = 'text'
                         THEN json_extract(snapshot_json, '$.query.logical_dataset') END,
                    CASE WHEN json_type(snapshot_json, '$.query.native_collection_id') = 'text'
                         THEN json_extract(snapshot_json, '$.query.native_collection_id') END,
                    CASE WHEN json_type(snapshot_json, '$.query.dataset') = 'text'
                         THEN json_extract(snapshot_json, '$.query.dataset') END,
                    json_array_length(snapshot_json, '$.candidates'),
                    -- Activation history is not recorded before schema 33;
                    -- derived deliveries and fallback re-searches are known.
                    CASE WHEN json_extract(snapshot_json, '$.query.derived_from_batch_id') IS NULL
                              AND batch_id NOT LIKE 'batch-fallback-%'
                         THEN 1 ELSE 0 END
             FROM search_batches
             WHERE json_valid(snapshot_json)
               AND json_type(snapshot_json, '$.query') = 'object'
               AND json_type(snapshot_json, '$.candidates') = 'array';",
        )?;
    }
    let mut skipped = 0;
    for table in [
        "task_snapshots",
        "task_runtime",
        "task_view_state",
        "task_membership_additions",
    ] {
        if !table_exists(connection, table)? {
            return Ok(skipped);
        }
    }
    let task_ids = {
        let mut statement = connection.prepare("SELECT task_id FROM task_snapshots")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids
    };
    for task_id in task_ids {
        let decoded = crate::repository::load_compact_task_connection(connection, &task_id)
            .and_then(|task| {
                let explicit =
                    crate::repository::load_explicit_task_source_item_ids(connection, &task_id)?
                        .is_some();
                Ok(task.map(|task| (task, explicit)))
            });
        match decoded {
            Ok(Some((task, explicit))) => {
                let indexed = index_task_items(
                    connection,
                    &task.snapshot,
                    explicit,
                    &task.runtime.selected_item_ids,
                );
                match indexed {
                    Ok(()) => {}
                    Err(error) if undecodable(&error) || matches!(error, StoreError::NotFound) => {
                        skipped += 1
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(None) => skipped += 1,
            Err(error) if undecodable(&error) => skipped += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(skipped)
}

fn source_view_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(StoreSourceView, Option<String>)> {
    Ok((
        StoreSourceView {
            resource_id: row.get(0)?,
            batch_id: row.get(1)?,
            item_count: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
            searched_at: row.get(3)?,
            pending_request_id: row.get(4)?,
            last_error: None,
            updated_at: row.get(6)?,
        },
        row.get(5)?,
    ))
}

fn valid_view_key(resource_id: &str, request_id: Option<&str>) -> Result<(), StoreError> {
    if resource_id.trim().is_empty() || resource_id.len() > 128 {
        return Err(StoreError::invalid("invalid source view resource ID"));
    }
    if request_id.is_some_and(|id| id.trim().is_empty() || id.len() > 128) {
        return Err(StoreError::invalid("invalid source view request ID"));
    }
    Ok(())
}

fn ensure_view_row(connection: &Connection, resource_id: &str, at: &str) -> Result<(), StoreError> {
    connection.execute(
        "INSERT OR IGNORE INTO source_views(resource_id, item_count, updated_at) VALUES (?1, 0, ?2)",
        params![resource_id, at],
    )?;
    Ok(())
}

impl TaskStore {
    /// Latest view-eligible (user search) frozen batch for each distinct
    /// provider/collection/selector key.
    /// The key set is small (one row per reviewed route), so Registry resolution
    /// in the service never scans candidate payloads or every batch.
    pub fn latest_frozen_batch_headers_per_route(
        &self,
    ) -> Result<Vec<StoreBatchRouteHeader>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT batch_id, provider_id, created_at, logical_dataset, native_collection_id,
                    dataset, candidate_count
             FROM (
                 SELECT h.*, ROW_NUMBER() OVER (
                     PARTITION BY provider_id, logical_dataset, native_collection_id, dataset
                     ORDER BY created_at DESC, batch_id DESC
                 ) AS position
                 FROM search_batch_headers h
                 WHERE view_eligible = 1
             )
             WHERE position = 1
             ORDER BY created_at DESC, batch_id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoreBatchRouteHeader {
                batch_id: row.get(0)?,
                provider_id: row.get(1)?,
                created_at: row.get(2)?,
                logical_dataset: row.get(3)?,
                native_collection_id: row.get(4)?,
                dataset: row.get(5)?,
                item_count: u64::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn load_source_views(&self) -> Result<Vec<StoreSourceView>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT resource_id, batch_id, item_count, searched_at, pending_request_id,
                    last_error_json, updated_at
             FROM source_views ORDER BY resource_id",
        )?;
        let rows = statement.query_map([], source_view_from_row)?;
        rows.map(|row| {
            let (mut view, error) = row?;
            view.last_error = error
                .map(|error| serde_json::from_str(&error).map_err(StoreError::Corrupt))
                .transpose()?;
            Ok(view)
        })
        .collect()
    }

    pub fn mark_source_view_pending(
        &mut self,
        resource_id: &str,
        request_id: &str,
        at: &str,
    ) -> Result<(), StoreError> {
        valid_view_key(resource_id, Some(request_id))?;
        let transaction = self.connection.transaction()?;
        ensure_view_row(&transaction, resource_id, at)?;
        transaction.execute(
            "UPDATE source_views SET pending_request_id = ?2, updated_at = ?3 WHERE resource_id = ?1",
            params![resource_id, request_id, at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Move the view to a published batch. The pointer only advances in
    /// (created_at, batch_id) order, so a late success for an older batch or a
    /// reconciliation never hides a newer result.
    pub fn record_source_view_success(
        &mut self,
        resource_id: &str,
        batch_id: &str,
        request_id: Option<&str>,
        at: &str,
    ) -> Result<(), StoreError> {
        valid_view_key(resource_id, request_id)?;
        let transaction = self.connection.transaction()?;
        let header: Option<(String, i64, bool)> = transaction
            .query_row(
                "SELECT created_at, candidate_count, view_eligible
                 FROM search_batch_headers WHERE batch_id = ?1",
                [batch_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (created_at, count, eligible) = header.ok_or(StoreError::NotFound)?;
        if !eligible {
            return Err(StoreError::invalid(
                "only a search publication can become a source view",
            ));
        }
        ensure_view_row(&transaction, resource_id, at)?;
        transaction.execute(
            "UPDATE source_views
             SET batch_id = ?2, searched_at = ?3, item_count = ?4, updated_at = ?5
             WHERE resource_id = ?1
               AND (batch_id IS NULL OR searched_at < ?3 OR (searched_at = ?3 AND batch_id < ?2))",
            params![resource_id, batch_id, created_at, count, at],
        )?;
        if let Some(request_id) = request_id {
            transaction.execute(
                "UPDATE source_views
                 SET last_error_json = NULL,
                     pending_request_id = CASE WHEN pending_request_id = ?2 THEN NULL
                                               ELSE pending_request_id END,
                     updated_at = ?3
                 WHERE resource_id = ?1",
                params![resource_id, request_id, at],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// A failed search records only its typed error; the batch pointer stays.
    pub fn record_source_view_failure(
        &mut self,
        resource_id: &str,
        request_id: &str,
        error: &TypedError,
        at: &str,
    ) -> Result<(), StoreError> {
        valid_view_key(resource_id, Some(request_id))?;
        let error_json = serde_json::to_string(error).map_err(StoreError::Corrupt)?;
        let transaction = self.connection.transaction()?;
        ensure_view_row(&transaction, resource_id, at)?;
        transaction.execute(
            "UPDATE source_views
             SET last_error_json = ?3,
                 pending_request_id = CASE WHEN pending_request_id = ?2 THEN NULL
                                           ELSE pending_request_id END,
                 updated_at = ?4
             WHERE resource_id = ?1",
            params![resource_id, request_id, error_json, at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Cancellation, or a pending request that did not survive a restart.
    pub fn clear_source_view_pending(
        &mut self,
        resource_id: &str,
        request_id: &str,
        at: &str,
    ) -> Result<(), StoreError> {
        valid_view_key(resource_id, Some(request_id))?;
        self.connection.execute(
            "UPDATE source_views SET pending_request_id = NULL, updated_at = ?3
             WHERE resource_id = ?1 AND pending_request_id = ?2",
            params![resource_id, request_id, at],
        )?;
        Ok(())
    }
}

/// Request to move selected products of a failed source-batch task onto the
/// matching items of a newer frozen batch, then retry the same task ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreRetryFallbackRequest {
    pub task_id: String,
    pub expected_revision: u64,
    pub refreshed_batch_id: String,
    pub item_ids: Vec<String>,
    pub event_id: String,
    pub occurred_at: String,
}

fn missing_table(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(_, Some(message)) if message.starts_with("no such table"))
}

/// Overlay the task's current snapshot revision: each product keeps its
/// item ID and runtime state, only its frozen item (assets, URLs) changes.
/// Earlier migrations run before schema 33 exists and see no revisions.
pub(crate) fn apply_task_item_rebinds(
    connection: &Connection,
    mut task: crate::StoredTask,
) -> Result<crate::StoredTask, StoreError> {
    let rows = connection
        .prepare_cached(
            "SELECT r.item_id, r.item_json FROM task_snapshot_heads h
             JOIN task_item_rebinds r
               ON r.task_id = h.task_id AND r.snapshot_revision <= h.snapshot_revision
             WHERE h.task_id = ?1
             ORDER BY r.snapshot_revision, r.item_id",
        )
        .and_then(|mut statement| {
            statement
                .query_map([&task.snapshot.task_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()
        });
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) if missing_table(&error) => return Ok(task),
        Err(error) => return Err(error.into()),
    };
    for (item_id, item_json) in rows {
        let item: insar_core::SnapshotItem =
            serde_json::from_str(&item_json).map_err(StoreError::Corrupt)?;
        if item.item_id != item_id {
            return Err(StoreError::invalid(
                "task item rebind changes the product ID",
            ));
        }
        let slot = task
            .snapshot
            .candidates
            .iter_mut()
            .find(|candidate| candidate.item_id == item_id)
            .ok_or_else(|| StoreError::invalid("task item rebind targets an unknown product"))?;
        *slot = item;
    }
    let orbit_events = connection
        .prepare_cached(
            "SELECT event_id, payload_json FROM task_events
             WHERE task_id = ?1
               AND (event_id LIKE 'orbit-resorb-%'
                    OR event_id LIKE 'orbit-upgrade-%'
                    OR event_id LIKE 'orbit-check-%'
                    OR event_id LIKE 'orbit-row-%')
             ORDER BY sequence ASC",
        )
        .and_then(|mut statement| {
            statement
                .query_map([&task.snapshot.task_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()
        });
    if let Ok(events) = orbit_events {
        for (event_id, payload_json) in events {
            if let Ok(req) =
                serde_json::from_str::<insar_core::StoreUpdateTaskRequest>(&payload_json)
            {
                let requested_type = if event_id.starts_with("orbit-resorb-") {
                    "RESORB"
                } else {
                    "POEORB"
                };
                for candidate in &mut task.snapshot.candidates {
                    if req
                        .command
                        .item_ids
                        .iter()
                        .any(|id| id == &candidate.item_id)
                    {
                        candidate.payload.insert(
                            "requested_orbit_type".to_owned(),
                            Value::String(requested_type.to_owned()),
                        );
                    }
                }
            }
        }
    }
    Ok(task)
}

/// Prefix of every batch frozen by a retry-fallback re-search.
pub const RETRY_FALLBACK_BATCH_PREFIX: &str = "batch-fallback-";

/// The task side of a retry fallback: the expected revision of a failed,
/// plain source-batch product task.
/// Derived optical delivery tasks preserve `derived_from_batch_id` plus a
/// known `delivery_kind` so retry-fallback can replay the underlying catalogue
/// search and regenerate fresh asset/URL metadata (such as expired MPC SAS URLs).
pub fn is_replayable_optical_delivery_kind(kind: &str) -> bool {
    matches!(
        kind,
        "landsat_c2_l2_asset_batch"
            | "landsat_c2_l2_bundle_batch"
            | "landsat_c2_l2_full_bundle"
            | "landsat_c2_l2_matched_full_bundle"
            | "landsat_c2_l2_complete_asset_set"
            | "landsat_c2_l2_asset_selection"
            | "sentinel2_l2a_analysis_band_batch"
            | "sentinel2_l2a_analysis_band_set"
            | "sentinel2_l1c_band_batch"
            | "sentinel2_cdse_safe_archive_batch"
            | "sentinel2_cdse_safe_archive"
            | "hls_multiband_batch"
            | "planetary_computer_reviewed_cog_assets"
    )
}

fn check_fallback_task(
    connection: &Connection,
    task: &crate::StoredTask,
    expected_revision: u64,
) -> Result<(), StoreError> {
    if task.runtime.revision != expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    if task.runtime.status != insar_core::TaskLifecycleStatus::Failed {
        return Err(StoreError::RetryFallbackTaskNotFailed);
    }
    let explicit =
        crate::repository::load_explicit_task_source_item_ids(connection, &task.snapshot.task_id)?
            .is_some();
    let derived_unsupported = query_text(&task.snapshot.query, "derived_from_batch_id").is_some()
        && !query_text(&task.snapshot.query, "delivery_kind")
            .is_some_and(is_replayable_optical_delivery_kind);
    if explicit
        || task.snapshot.operation.is_some()
        || task.snapshot.orbit_association.is_some()
        || derived_unsupported
    {
        return Err(StoreError::RetryFallbackUnsupported);
    }
    Ok(())
}

/// The batch side: a re-search frozen for this failure (fallback prefix,
/// created after the task last changed) with the task's frozen conditions.
fn check_fallback_batch(
    connection: &Connection,
    task: &crate::StoredTask,
    batch: &FrozenSearchBatch,
) -> Result<(), StoreError> {
    let after_failure: Option<bool> = connection.query_row(
        "SELECT julianday(?1) > julianday(?2)",
        params![batch.created_at, task.runtime.updated_at],
        |row| row.get(0),
    )?;
    if batch.batch_id.len() <= RETRY_FALLBACK_BATCH_PREFIX.len()
        || !batch.batch_id.starts_with(RETRY_FALLBACK_BATCH_PREFIX)
        || after_failure != Some(true)
        || batch.provider_id != task.snapshot.provider_id
        || batch.query != task.snapshot.query
        || batch.aoi_geojson != task.snapshot.aoi_geojson
    {
        return Err(StoreError::RetryFallbackConfirmationMismatch);
    }
    Ok(())
}

impl TaskStore {
    /// Freeze a retry-fallback re-search batch for a failed task and record it
    /// as the only batch a rebuild of that task revision may use.
    pub fn offer_retry_fallback_batch(
        &mut self,
        task_id: &str,
        expected_revision: u64,
        request: &insar_core::StoreSearchBatchRequest,
    ) -> Result<(), StoreError> {
        if request.activate_workbench {
            return Err(StoreError::invalid(
                "a retry-fallback batch never activates the Workbench",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let task = crate::repository::load_compact_task_connection(&transaction, task_id)?
            .ok_or(StoreError::NotFound)?;
        check_fallback_task(&transaction, &task, expected_revision)?;
        check_fallback_batch(&transaction, &task, &request.batch)?;
        crate::repository::save_search_batch_in_transaction(&transaction, request)?;
        transaction.execute(
            "INSERT INTO task_snapshot_heads(
                 task_id, snapshot_revision, source_batch_id, offered_batch_id,
                 offered_task_revision, updated_at
             ) VALUES (?1, 0, ?2, ?3, ?4, ?5)
             ON CONFLICT(task_id) DO UPDATE SET
                 offered_batch_id = excluded.offered_batch_id,
                 offered_task_revision = excluded.offered_task_revision,
                 updated_at = excluded.updated_at",
            params![
                task_id,
                task.snapshot.batch_id,
                request.batch.batch_id,
                crate::repository::sqlite_integer(expected_revision, "task revision")?,
                request.batch.created_at,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// The batch a retry-fallback rebuild of this task must name, with the
    /// task revision it was frozen for.
    pub fn retry_fallback_offer(&self, task_id: &str) -> Result<Option<(String, u64)>, StoreError> {
        let offer: Option<(Option<String>, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT offered_batch_id, offered_task_revision FROM task_snapshot_heads
                 WHERE task_id = ?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(match offer {
            Some((Some(batch_id), Some(revision))) => {
                Some((batch_id, u64::try_from(revision).unwrap_or(0)))
            }
            _ => None,
        })
    }
}

impl TaskStore {
    /// Current snapshot revision of a task: 0 is the original frozen snapshot.
    pub fn task_snapshot_revision(&self, task_id: &str) -> Result<u64, StoreError> {
        let revision: Option<i64> = self
            .connection
            .query_row(
                "SELECT snapshot_revision FROM task_snapshot_heads WHERE task_id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(revision.map_or(0, |revision| u64::try_from(revision).unwrap_or(0)))
    }

    /// Rebuild selected products of a failed source-batch task from a newer
    /// frozen batch with identical frozen conditions, then retry the same task
    /// ID, all in one IMMEDIATE transaction. The original task snapshot and
    /// both batches stay immutable: the new items are a new snapshot revision
    /// and the task's head pointer moves to it.
    pub fn rebind_task_items_and_retry(
        &mut self,
        request: &StoreRetryFallbackRequest,
    ) -> Result<crate::StoredTask, StoreError> {
        let unique = request
            .item_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if request.task_id.trim().is_empty()
            || request.refreshed_batch_id.trim().is_empty()
            || request.event_id.trim().is_empty()
            || request.event_id.len() > 192
            || request.occurred_at.trim().is_empty()
            || request.item_ids.is_empty()
            || unique.len() != request.item_ids.len()
        {
            return Err(StoreError::invalid("retry fallback request is invalid"));
        }
        let retry = insar_core::StoreUpdateTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: request.expected_revision,
            command: insar_core::StoreTaskCommand {
                kind: insar_core::TaskStateCommandKind::RetryTask,
                item_ids: Vec::new(),
            },
            event_id: request.event_id.clone(),
            occurred_at: request.occurred_at.clone(),
        };
        retry.validate().map_err(StoreError::invalid)?;
        let retry_json = serde_json::to_string(&retry).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let replay: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_events WHERE event_id = ?1)",
            [&request.event_id],
            |row| row.get(0),
        )?;
        if !replay {
            let task =
                crate::repository::load_compact_task_connection(&transaction, &request.task_id)?
                    .ok_or(StoreError::NotFound)?;
            check_fallback_task(&transaction, &task, request.expected_revision)?;
            let refreshed =
                crate::repository::load_batch(&transaction, &request.refreshed_batch_id)?
                    .ok_or(StoreError::RetryFallbackConfirmationMismatch)?;
            check_fallback_batch(&transaction, &task, &refreshed)?;
            let offered: Option<(Option<String>, Option<i64>)> = transaction
                .query_row(
                    "SELECT offered_batch_id, offered_task_revision FROM task_snapshot_heads
                     WHERE task_id = ?1",
                    [&request.task_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let revision =
                crate::repository::sqlite_integer(task.runtime.revision, "task revision")?;
            if offered != Some((Some(refreshed.batch_id.clone()), Some(revision))) {
                return Err(StoreError::RetryFallbackConfirmationMismatch);
            }
            // Only products the retry will run again are rebuilt; completed
            // or skipped products keep their frozen item.
            let retryable = task
                .runtime
                .failed_item_ids
                .iter()
                .chain(&task.runtime.cancelled_item_ids)
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let mut items = Vec::with_capacity(request.item_ids.len());
            for item_id in &request.item_ids {
                if !retryable.contains(item_id.as_str()) {
                    return Err(StoreError::RetryFallbackItemNotRetryable);
                }
                let item = refreshed
                    .candidates
                    .iter()
                    .find(|candidate| &candidate.item_id == item_id)
                    .ok_or_else(|| {
                        StoreError::invalid("retry fallback product is not in the refreshed batch")
                    })?;
                item.validate().map_err(StoreError::invalid)?;
                items.push(item);
            }
            let next: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(snapshot_revision), 0) + 1 FROM task_snapshot_heads WHERE task_id = ?1",
                [&request.task_id],
                |row| row.get(0),
            )?;
            for item in items {
                transaction.execute(
                    "INSERT INTO task_item_rebinds(
                         task_id, snapshot_revision, item_id, source_batch_id, item_json, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        request.task_id,
                        next,
                        item.item_id,
                        refreshed.batch_id,
                        serde_json::to_string(item).map_err(StoreError::Corrupt)?,
                        request.occurred_at,
                    ],
                )?;
                transaction.execute(
                    "INSERT OR IGNORE INTO batch_task_items(
                         source_batch_id, source_item_id, task_id, task_item_id, relation
                     ) VALUES (?1, ?2, ?3, ?2, 'direct')",
                    params![refreshed.batch_id, item.item_id, request.task_id],
                )?;
            }
            transaction.execute(
                "INSERT INTO task_snapshot_heads(task_id, snapshot_revision, source_batch_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(task_id) DO UPDATE SET
                     snapshot_revision = excluded.snapshot_revision,
                     source_batch_id = excluded.source_batch_id,
                     offered_batch_id = NULL,
                     offered_task_revision = NULL,
                     updated_at = excluded.updated_at",
                params![request.task_id, next, refreshed.batch_id, request.occurred_at],
            )?;
        }
        let task = crate::runtime::update_task_in_transaction(&transaction, &retry, &retry_json)?;
        transaction.commit()?;
        Ok(task)
    }
}
