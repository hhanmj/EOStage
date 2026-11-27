//! #22 B slice 5 (Owner decision 2026-11-03, option A): while a search batch
//! is still loading, rows that have *arrived* (are staged) can be checked and
//! downloaded row by row; select-all over the batch waits for its seal.
//!
//! A task snapshot must reference a frozen `search_batches` row, and the
//! loading batch is written there exactly once, by its seal. So admission of
//! arrived rows freezes exactly those staged rows into a small derived batch
//! (`delivery_kind = progressive_admission`, `derived_from_batch_id` = the
//! loading batch) and creates the task against it in one transaction. Every
//! derived candidate carries `metadata.source_item_id`, so the existing
//! reverse index links the task back to the loading batch (`derived`
//! relation): the loading batch's pages show the row state while it loads and
//! after it seals under the same batch id, and admission deduplicates against
//! those links ([`TaskStore::active_task_for_products`]).
//!
//! A row that has not arrived is never frozen: the read refuses it with
//! [`StoreError::RowsNotYetLoaded`], and the write transaction re-checks every
//! derived candidate byte-for-byte against the staged (or, after a racing
//! seal, the sealed) row before anything is written.

use std::collections::HashMap;

use insar_core::{
    FrozenSearchBatch, FrozenTaskSnapshot, SearchWorkbenchState, SnapshotItem,
    StoreCreateTaskRequest, StoreSearchBatchRequest,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;

use crate::repository::StoredTask;
use crate::{StoreError, TaskStore, MAX_WORKBENCH_PAGE_SIZE};

/// `query.delivery_kind` of a batch frozen from arrived rows of a loading
/// batch. Admission deduplication follows `derived` links only for tasks
/// frozen from such a batch, so every other derived delivery keeps its
/// existing semantics.
pub const PROGRESSIVE_ADMISSION_DELIVERY_KIND: &str = "progressive_admission";

const DERIVED_FROM_KEY: &str = "derived_from_batch_id";
const DELIVERY_KIND_KEY: &str = "delivery_kind";
const SOURCE_ITEM_ID_KEY: &str = "source_item_id";

/// Whether a task snapshot query names a progressive admission batch.
pub(crate) fn is_progressive_admission_query(query: &serde_json::Map<String, Value>) -> bool {
    query.get(DELIVERY_KIND_KEY).and_then(Value::as_str)
        == Some(PROGRESSIVE_ADMISSION_DELIVERY_KIND)
}

/// Resolve a progressive task to its immutable, sealed original batch. The
/// derived header and every initially frozen source candidate must agree with
/// that batch before callers use its other rows for an operation.
pub(crate) fn verified_progressive_source(
    connection: &Connection,
    snapshot: &FrozenTaskSnapshot,
) -> Result<Option<String>, StoreError> {
    if !is_progressive_admission_query(&snapshot.query) {
        return Ok(None);
    }
    let source_id = snapshot
        .query
        .get(DERIVED_FROM_KEY)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && *id != snapshot.batch_id)
        .ok_or_else(|| StoreError::invalid("progressive task lacks its source batch"))?;
    if !crate::batch_progress::batch_is_sealed(connection, source_id)? {
        return Err(crate::batch_progress::missing_batch_error(
            connection, source_id,
        ));
    }
    let header_json: String = connection
        .query_row(
            "SELECT json_set(snapshot_json, '$.candidates', json('[]')) FROM search_batches \
         WHERE batch_id = ?1 AND json_type(snapshot_json, '$.candidates') = 'array'",
            [source_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| StoreError::invalid("progressive source header is invalid"))?;
    let source: FrozenSearchBatch =
        serde_json::from_str(&header_json).map_err(StoreError::Corrupt)?;
    source.validate().map_err(StoreError::invalid)?;
    let derived = crate::repository::load_batch(connection, &snapshot.batch_id)?
        .ok_or_else(|| StoreError::invalid("progressive admission batch is missing"))?;
    if derived.candidates.is_empty() || derived.candidates.len() > MAX_WORKBENCH_PAGE_SIZE as usize
    {
        return Err(StoreError::invalid(
            "progressive admission candidate count is invalid",
        ));
    }
    let mut expected = source.clone();
    expected.batch_id = derived.batch_id.clone();
    expected.created_at = derived.created_at.clone();
    expected.search_stats = None;
    expected.query.insert(
        DERIVED_FROM_KEY.to_owned(),
        Value::String(source_id.to_owned()),
    );
    expected.query.insert(
        DELIVERY_KIND_KEY.to_owned(),
        Value::String(PROGRESSIVE_ADMISSION_DELIVERY_KIND.to_owned()),
    );
    let mut header = derived.clone();
    header.candidates.clear();
    if header != expected
        || source.batch_id != source_id
        || snapshot.provider_id != derived.provider_id
        || snapshot.schema_version != derived.schema_version
        || snapshot.aoi_name != derived.aoi_name
        || snapshot.aoi_geojson != derived.aoi_geojson
        || snapshot.query != derived.query
        || source.query.contains_key(DERIVED_FROM_KEY)
        || source.query.contains_key(DELIVERY_KIND_KEY)
    {
        return Err(StoreError::invalid(
            "progressive task source binding is invalid",
        ));
    }
    let ids: Vec<String> = derived
        .candidates
        .iter()
        .map(|item| item.item_id.clone())
        .collect();
    validate_item_ids(&ids)?;
    let rows = crate::batch_candidates::load_batch_candidate_items_connection(
        connection, source_id, &ids,
    )?
    .ok_or(StoreError::NotFound)?;
    if rows.len() != derived.candidates.len() {
        return Err(StoreError::invalid(
            "progressive admission source rows are missing",
        ));
    }
    let originals: HashMap<&str, &SnapshotItem> = rows
        .iter()
        .map(|item| (item.item_id.as_str(), item))
        .collect();
    for item in &derived.candidates {
        let mut unlinked = item.clone();
        if unlinked
            .metadata
            .remove(SOURCE_ITEM_ID_KEY)
            .as_ref()
            .and_then(Value::as_str)
            != Some(item.item_id.as_str())
            || originals.get(item.item_id.as_str()).copied() != Some(&unlinked)
        {
            return Err(StoreError::invalid(
                "progressive admission source row differs",
            ));
        }
    }
    Ok(Some(source_id.to_owned()))
}

impl TaskStore {
    /// Original batch for a progressive task, after the source is sealed and
    /// the persisted binding has been checked. Other tasks return None.
    pub fn progressive_task_source_batch_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, StoreError> {
        let task = crate::repository::load_compact_task_connection(&self.connection, task_id)?
            .ok_or(StoreError::NotFound)?;
        verified_progressive_source(&self.connection, &task.snapshot)
    }
}

fn requested_ids_json(item_ids: &[String]) -> Result<String, StoreError> {
    serde_json::to_string(item_ids).map_err(StoreError::Corrupt)
}

/// The staged JSON of `item_ids` for a loading request, keyed by item id.
fn staged_rows(
    connection: &Connection,
    request_id: &str,
    item_ids: &[String],
) -> Result<HashMap<String, String>, StoreError> {
    let mut statement = connection.prepare_cached(
        "SELECT item_id, item_json FROM search_batch_staging_candidates \
         WHERE request_id = ?1 AND item_id IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = statement
        .query_map(params![request_id, requested_ids_json(item_ids)?], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// The sealed JSON of `item_ids` in a batch that sealed after the admission
/// batch was read (the progressive seal always writes candidate rows).
fn sealed_rows(
    connection: &Connection,
    batch_id: &str,
    item_ids: &[String],
) -> Result<HashMap<String, String>, StoreError> {
    if !matches!(
        crate::batch_candidates::candidate_rows_kind_connection(connection, batch_id)?,
        Some(crate::batch_candidates::StoreCandidateKind::Rows)
    ) {
        return Err(StoreError::invalid(
            "sealed progressive batch candidate rows are missing",
        ));
    }
    let rows = crate::batch_candidates::load_batch_candidate_items_connection(
        connection, batch_id, item_ids,
    )?
    .ok_or(StoreError::NotFound)?;
    rows.into_iter()
        .map(|item| {
            let json = serde_json::to_string(&item).map_err(StoreError::Corrupt)?;
            Ok((item.item_id, json))
        })
        .collect()
}

fn decode_item(json: &str) -> Result<SnapshotItem, StoreError> {
    let item: SnapshotItem = serde_json::from_str(json).map_err(StoreError::Corrupt)?;
    item.validate().map_err(StoreError::invalid)?;
    Ok(item)
}

fn validate_item_ids(item_ids: &[String]) -> Result<(), StoreError> {
    if item_ids.is_empty() || item_ids.len() > MAX_WORKBENCH_PAGE_SIZE as usize {
        return Err(StoreError::invalid(
            "a loading admission names between 1 and 250 rows",
        ));
    }
    let unique = item_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    if unique.len() != item_ids.len() || item_ids.iter().any(|id| id.trim().is_empty()) {
        return Err(StoreError::invalid(
            "loading admission row IDs must be unique and non-empty",
        ));
    }
    Ok(())
}

impl TaskStore {
    /// The admission batch for exact arrived rows of a loading batch, built
    /// in memory (nothing is written). `Ok(None)` when `loading_batch_id` is
    /// not loading (sealed, never published or unknown): the caller keeps the
    /// classic frozen-batch path. A row that has not arrived is the typed
    /// [`StoreError::RowsNotYetLoaded`]. Candidates are in the frozen
    /// (item-ID) order and each carries `metadata.source_item_id`.
    pub fn loading_admission_batch(
        &self,
        loading_batch_id: &str,
        item_ids: &[String],
        admission_batch_id: &str,
        created_at: &str,
    ) -> Result<Option<FrozenSearchBatch>, StoreError> {
        validate_item_ids(item_ids)?;
        let Some(request_id) =
            crate::batch_progress::loading_request_for_batch(&self.connection, loading_batch_id)?
        else {
            return Ok(None);
        };
        let header_json: String = self
            .connection
            .query_row(
                "SELECT batch_header_json FROM search_batch_staging WHERE request_id = ?1",
                [&request_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let mut batch: FrozenSearchBatch =
            serde_json::from_str(&header_json).map_err(StoreError::Corrupt)?;
        if batch.query.contains_key(DERIVED_FROM_KEY) || batch.query.contains_key(DELIVERY_KIND_KEY)
        {
            return Err(StoreError::invalid(
                "loading batch uses reserved admission query fields",
            ));
        }
        let rows = staged_rows(&self.connection, &request_id, item_ids)?;
        if rows.len() != item_ids.len() {
            return Err(StoreError::RowsNotYetLoaded {
                batch_id: loading_batch_id.to_owned(),
            });
        }
        let mut sorted: Vec<&String> = item_ids.iter().collect();
        sorted.sort();
        let mut candidates = Vec::with_capacity(sorted.len());
        for item_id in sorted {
            let mut item = decode_item(&rows[item_id])?;
            if item.metadata.contains_key(SOURCE_ITEM_ID_KEY) {
                return Err(StoreError::invalid(
                    "loading batch row uses reserved admission source field",
                ));
            }
            item.metadata.insert(
                SOURCE_ITEM_ID_KEY.to_owned(),
                Value::String(item.item_id.clone()),
            );
            candidates.push(item);
        }
        batch.batch_id = admission_batch_id.to_owned();
        batch.created_at = created_at.to_owned();
        batch.search_stats = None;
        batch.query.insert(
            DERIVED_FROM_KEY.to_owned(),
            Value::String(loading_batch_id.to_owned()),
        );
        batch.query.insert(
            DELIVERY_KIND_KEY.to_owned(),
            Value::String(PROGRESSIVE_ADMISSION_DELIVERY_KIND.to_owned()),
        );
        batch.candidates = candidates;
        batch.validate().map_err(StoreError::invalid)?;
        Ok(Some(batch))
    }

    /// Atomically freeze an admission batch from
    /// [`Self::loading_admission_batch`] and create its task. Inside the write
    /// transaction every candidate (without its `source_item_id` link) must
    /// equal the row still staged for the loading batch, or the row its seal
    /// wrote if it sealed meanwhile; otherwise nothing is written. `explicit`
    /// carries the source product ids and task items of an explicit (burst
    /// asset) task.
    pub fn create_task_from_loading_admission(
        &mut self,
        loading_batch_id: &str,
        admission: &FrozenSearchBatch,
        request: &StoreCreateTaskRequest,
        explicit: Option<(&[String], &[SnapshotItem])>,
    ) -> Result<StoredTask, StoreError> {
        if admission
            .query
            .get(DERIVED_FROM_KEY)
            .and_then(Value::as_str)
            != Some(loading_batch_id)
            || !is_progressive_admission_query(&admission.query)
            || admission.batch_id == loading_batch_id
            || request.batch_id != admission.batch_id
            || admission.candidates.is_empty()
        {
            return Err(StoreError::invalid(
                "a loading admission batch must derive from its loading batch",
            ));
        }
        let item_ids: Vec<String> = admission
            .candidates
            .iter()
            .map(|item| item.item_id.clone())
            .collect();
        validate_item_ids(&item_ids)?;
        let admission_ids = item_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let requested_sources =
            explicit.map_or(request.selected_item_ids.as_slice(), |(ids, _)| ids);
        validate_item_ids(requested_sources)?;
        if requested_sources
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>()
            != admission_ids
            || (explicit.is_none()
                && request
                    .checked_item_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>()
                    != admission_ids)
        {
            return Err(StoreError::invalid(
                "loading admission must match the selected sources",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (rows, source_header_json) =
            match crate::batch_progress::loading_request_for_batch(&transaction, loading_batch_id)?
            {
                Some(request_id) => {
                    let header: String = transaction.query_row(
                        "SELECT batch_header_json FROM search_batch_staging WHERE request_id = ?1",
                        [&request_id],
                        |row| row.get(0),
                    )?;
                    (staged_rows(&transaction, &request_id, &item_ids)?, header)
                }
                None if crate::batch_progress::batch_is_sealed(&transaction, loading_batch_id)? => {
                    let (length, header): (Option<i64>, String) = transaction.query_row(
                        "SELECT json_array_length(snapshot_json, '$.candidates'), \
                         json_set(snapshot_json, '$.candidates', json('[]')) \
                         FROM search_batches WHERE batch_id = ?1",
                        [loading_batch_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;
                    if length.is_none() {
                        return Err(StoreError::invalid(
                            "sealed batch candidate array is invalid",
                        ));
                    }
                    (
                        sealed_rows(&transaction, loading_batch_id, &item_ids)?,
                        header,
                    )
                }
                None => return Err(StoreError::NotFound),
            };
        let mut expected: FrozenSearchBatch =
            serde_json::from_str(&source_header_json).map_err(StoreError::Corrupt)?;
        if expected.batch_id != loading_batch_id
            || expected.query.contains_key(DERIVED_FROM_KEY)
            || expected.query.contains_key(DELIVERY_KIND_KEY)
        {
            return Err(StoreError::invalid(
                "invalid source batch for loading admission",
            ));
        }
        expected.batch_id = admission.batch_id.clone();
        expected.created_at = admission.created_at.clone();
        expected.search_stats = None;
        expected.candidates.clear();
        expected.query.insert(
            DERIVED_FROM_KEY.to_owned(),
            Value::String(loading_batch_id.to_owned()),
        );
        expected.query.insert(
            DELIVERY_KIND_KEY.to_owned(),
            Value::String(PROGRESSIVE_ADMISSION_DELIVERY_KIND.to_owned()),
        );
        let mut actual = admission.clone();
        actual.candidates.clear();
        if actual != expected {
            return Err(StoreError::invalid(
                "loading admission header differs from its source",
            ));
        }
        let requested_json = serde_json::to_string(&item_ids).map_err(StoreError::Corrupt)?;
        let conflict: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM batch_task_items i JOIN task_runtime r ON r.task_id = i.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = i.task_id WHERE i.source_batch_id = ?1 \
             AND i.source_item_id IN (SELECT value FROM json_each(?2)) AND i.task_id != ?3 \
             AND d.task_id IS NULL AND r.status IN ('queued','running','paused','interrupted'))",
            params![loading_batch_id, requested_json, request.task_id], |row| row.get(0),
        )?;
        if conflict {
            return Err(StoreError::PartialActiveTaskOverlap);
        }
        for candidate in &admission.candidates {
            let Some(json) = rows.get(&candidate.item_id) else {
                return Err(StoreError::RowsNotYetLoaded {
                    batch_id: loading_batch_id.to_owned(),
                });
            };
            let mut unlinked = candidate.clone();
            if unlinked
                .metadata
                .remove(SOURCE_ITEM_ID_KEY)
                .and_then(|value| value.as_str().map(str::to_owned))
                .as_deref()
                != Some(candidate.item_id.as_str())
            {
                return Err(StoreError::invalid(
                    "a loading admission row must link to itself",
                ));
            }
            if decode_item(json)? != unlinked {
                return Err(StoreError::invalid(
                    "a loading admission row differs from the arrived row",
                ));
            }
        }
        let parents = item_ids
            .iter()
            .map(|id| {
                rows.get(id)
                    .ok_or_else(|| StoreError::invalid("loading source row is missing"))
                    .and_then(|json| decode_item(json))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let contains_burst = parents.iter().any(|item| {
            item.provider_id == "asf.catalog"
                && item
                    .metadata
                    .get("dataset")
                    .and_then(Value::as_str)
                    .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"))
        });
        if contains_burst && explicit.is_none() {
            return Err(StoreError::invalid(
                "loading Burst requires explicit assets",
            ));
        }
        if let Some((_, task_items)) = explicit {
            // The progressive explicit path is the unified Burst All policy.
            // Derive executable assets again from transaction-checked rows.
            if request.layout != insar_core::DownloadLayout::Direct {
                return Err(StoreError::invalid("loading Burst requires direct layout"));
            }
            insar_core::validate_burst_task_items_for_policy(
                &parents,
                insar_core::BurstAssetPolicy::All,
                task_items,
            )
            .map_err(|_| StoreError::invalid("loading Burst assets differ from source"))?;
        }
        crate::repository::save_search_batch_in_transaction(
            &transaction,
            &StoreSearchBatchRequest {
                batch: admission.clone(),
                workbench: SearchWorkbenchState {
                    batch_id: admission.batch_id.clone(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: request.page_size,
                    revision: 0,
                },
                activate_workbench: false,
            },
        )?;
        let stored = match explicit {
            Some((source_item_ids, task_items)) => {
                crate::repository::create_task_with_explicit_items_in_transaction(
                    &transaction,
                    request,
                    source_item_ids,
                    task_items,
                    None,
                    None,
                )?
            }
            None => {
                crate::repository::create_task_in_transaction(&transaction, request, None, None)?
            }
        };
        transaction.commit()?;
        Ok(stored)
    }
}
