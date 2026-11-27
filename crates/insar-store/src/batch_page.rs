use std::collections::{HashMap, HashSet};

use insar_core::{
    DownloadItemStatus, RowTaskAction, RowTaskState, SearchBatchProgressState, SnapshotItem,
    StoreBatchPage, StoreBatchPageRequest, TaskLifecycleStatus,
};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::batch_candidates::{
    candidate_rows_kind_connection, StoreBatchSelection, StoreCandidateKind,
};
use crate::core_restore::{
    aggregate_task_item_status, task_page_item_status, MAX_WORKBENCH_PAGE_SIZE,
};
use crate::repository::{
    load_compact_task_connection, load_explicit_task_source_item_ids, StoredTask,
};
use crate::{StoreError, TaskStore};

fn is_live(status: &TaskLifecycleStatus) -> bool {
    !matches!(
        status,
        TaskLifecycleStatus::Completed
            | TaskLifecycleStatus::Failed
            | TaskLifecycleStatus::Cancelled
    )
}

fn check_task_coverage(
    task: &StoredTask,
    selected: &HashSet<&str>,
    expected_assets: Option<&HashSet<(&str, &str)>>,
    use_backlink: bool,
) -> Result<bool, StoreError> {
    let items = task
        .snapshot
        .candidates
        .iter()
        .filter(|item| task.runtime.selected_item_ids.contains(&item.item_id));
    let covered = items
        .clone()
        .filter_map(|item| {
            if use_backlink {
                item.metadata.get("source_item_id").and_then(Value::as_str)
            } else {
                Some(item.item_id.as_str())
            }
        })
        .collect::<HashSet<_>>();
    if selected.is_disjoint(&covered) {
        return Ok(false);
    }
    let assets = items
        .filter_map(|item| {
            item.metadata
                .get("source_item_id")
                .and_then(Value::as_str)
                .map(|source| (source, item.item_id.as_str()))
        })
        .collect::<HashSet<_>>();
    if !selected.is_subset(&covered)
        || expected_assets.is_some_and(|expected| !expected.is_subset(&assets))
    {
        return Err(StoreError::PartialActiveTaskOverlap);
    }
    Ok(true)
}

impl TaskStore {
    /// Exact product membership across the whole frozen batch, independent of
    /// pagination. Reuse a live task only when it covers the entire request;
    /// a partial overlap is refused so no requested products disappear silently.
    pub fn active_task_for_products(
        &self,
        batch_id: &str,
        item_ids: &[String],
    ) -> Result<Option<StoredTask>, StoreError> {
        self.active_task_for_products_inner(batch_id, item_ids, None)
    }

    /// Reuse an explicit-asset task only if it covers every expected source
    /// product and asset pair, not merely one asset of each source product.
    pub fn active_task_for_products_with_assets(
        &self,
        batch_id: &str,
        item_ids: &[String],
        expected_items: &[SnapshotItem],
    ) -> Result<Option<StoredTask>, StoreError> {
        let selected = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let mut expected = HashSet::new();
        for item in expected_items {
            let source = item
                .metadata
                .get("source_item_id")
                .and_then(Value::as_str)
                .filter(|source| selected.contains(source))
                .ok_or_else(|| StoreError::invalid("expected asset lacks its selected source"))?;
            if !expected.insert((source, item.item_id.as_str())) {
                return Err(StoreError::invalid(
                    "expected asset identities must be unique",
                ));
            }
        }
        let sources = expected
            .iter()
            .map(|(source, _)| *source)
            .collect::<HashSet<_>>();
        if expected.is_empty() || sources != selected {
            return Err(StoreError::invalid(
                "expected assets must cover every selected source",
            ));
        }
        self.active_task_for_products_inner(batch_id, item_ids, Some(&expected))
    }

    fn active_task_for_products_inner(
        &self,
        batch_id: &str,
        item_ids: &[String],
        expected_assets: Option<&HashSet<(&str, &str)>>,
    ) -> Result<Option<StoredTask>, StoreError> {
        let selected = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let requested = serde_json::to_string(item_ids).map_err(StoreError::Corrupt)?;
        // Schema-33 reverse index: only tasks frozen from this batch itself
        // (direct or explicit files), never derived tasks of other batches.
        let mut stmt = self.connection.prepare(
            "SELECT DISTINCT s.task_id, s.created_at FROM batch_task_items i \
             JOIN task_snapshots s ON s.task_id = i.task_id \
             JOIN task_runtime r ON r.task_id = i.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = i.task_id \
             WHERE i.source_batch_id = ?1 AND i.relation IN ('direct','explicit') \
               AND i.source_item_id IN (SELECT value FROM json_each(?2)) \
               AND d.task_id IS NULL AND r.status IN ('queued','running','paused','interrupted') \
             ORDER BY s.created_at DESC, s.task_id DESC",
        )?;
        let ids = stmt
            .query_map(params![batch_id, requested], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in ids {
            let Some(task) = load_compact_task_connection(&self.connection, &id)? else {
                continue;
            };
            let explicit = load_explicit_task_source_item_ids(&self.connection, &id)?.is_some();
            if !is_live(&task.runtime.status) {
                continue;
            }
            if check_task_coverage(&task, &selected, expected_assets, explicit)? {
                return Ok(Some(task));
            }
        }
        self.active_progressive_admission_task(batch_id, &selected, &requested, expected_assets)
    }

    /// #22 B slice 5: a live task frozen from arrived rows of `batch_id`
    /// while it was loading links back through a `derived` reverse-index
    /// row. Only tasks of a progressive admission batch are followed, so a
    /// batch without such tasks keeps exactly the direct/explicit answer.
    fn active_progressive_admission_task(
        &self,
        batch_id: &str,
        selected: &HashSet<&str>,
        requested: &str,
        expected_assets: Option<&HashSet<(&str, &str)>>,
    ) -> Result<Option<StoredTask>, StoreError> {
        let mut stmt = self.connection.prepare(
            "SELECT DISTINCT s.task_id, s.created_at FROM batch_task_items i \
             JOIN task_snapshots s ON s.task_id = i.task_id \
             JOIN task_runtime r ON r.task_id = i.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = i.task_id \
             WHERE i.source_batch_id = ?1 AND i.relation = 'derived' \
               AND i.source_item_id IN (SELECT value FROM json_each(?2)) \
               AND d.task_id IS NULL AND r.status IN ('queued','running','paused','interrupted') \
             ORDER BY s.created_at DESC, s.task_id DESC",
        )?;
        let ids = stmt
            .query_map(params![batch_id, requested], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in ids {
            let Some(task) = load_compact_task_connection(&self.connection, &id)? else {
                continue;
            };
            if !is_live(&task.runtime.status)
                || !crate::loading_admission::is_progressive_admission_query(&task.snapshot.query)
            {
                continue;
            }
            if check_task_coverage(&task, selected, expected_assets, true)? {
                return Ok(Some(task));
            }
        }
        Ok(None)
    }

    /// All source candidate `item_id`s of `batch_id` that are currently
    /// covered by at least one live (`queued`/`running`/`paused`/`interrupted`)
    /// non-tombstoned task (including direct, explicit-asset, and progressive
    /// loading-admission tasks). Used by #22 B slice 4 all-match preview and
    /// admission to account for already-active products as `skipped` while
    /// admitting the remaining uncovered products.
    pub fn active_covered_items_for_batch(
        &self,
        batch_id: &str,
    ) -> Result<HashSet<String>, StoreError> {
        let mut covered = HashSet::new();
        let mut stmt = self.connection.prepare(
            "SELECT DISTINCT s.task_id, s.created_at FROM batch_task_items i \
             JOIN task_snapshots s ON s.task_id = i.task_id \
             JOIN task_runtime r ON r.task_id = i.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = i.task_id \
             WHERE i.source_batch_id = ?1 AND i.relation IN ('direct','explicit') \
               AND d.task_id IS NULL AND r.status IN ('queued','running','paused','interrupted') \
             ORDER BY s.created_at DESC, s.task_id DESC",
        )?;
        let direct_ids = stmt
            .query_map([batch_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in direct_ids {
            let Some(task) = load_compact_task_connection(&self.connection, &id)? else {
                continue;
            };
            if !is_live(&task.runtime.status) {
                continue;
            }
            let explicit = load_explicit_task_source_item_ids(&self.connection, &id)?.is_some();
            for item in task
                .snapshot
                .candidates
                .iter()
                .filter(|item| task.runtime.selected_item_ids.contains(&item.item_id))
            {
                let source_id = if explicit {
                    item.metadata.get("source_item_id").and_then(Value::as_str)
                } else {
                    Some(item.item_id.as_str())
                };
                if let Some(source_id) = source_id {
                    covered.insert(source_id.to_owned());
                }
            }
        }

        let mut derived_stmt = self.connection.prepare(
            "SELECT DISTINCT s.task_id, s.created_at FROM batch_task_items i \
             JOIN task_snapshots s ON s.task_id = i.task_id \
             JOIN task_runtime r ON r.task_id = i.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = i.task_id \
             WHERE i.source_batch_id = ?1 AND i.relation = 'derived' \
               AND d.task_id IS NULL AND r.status IN ('queued','running','paused','interrupted') \
             ORDER BY s.created_at DESC, s.task_id DESC",
        )?;
        let derived_ids = derived_stmt
            .query_map([batch_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in derived_ids {
            let Some(task) = load_compact_task_connection(&self.connection, &id)? else {
                continue;
            };
            if !is_live(&task.runtime.status)
                || !crate::loading_admission::is_progressive_admission_query(&task.snapshot.query)
            {
                continue;
            }
            for item in task
                .snapshot
                .candidates
                .iter()
                .filter(|item| task.runtime.selected_item_ids.contains(&item.item_id))
            {
                if let Some(source_id) = item.metadata.get("source_item_id").and_then(Value::as_str)
                {
                    covered.insert(source_id.to_owned());
                }
            }
        }
        Ok(covered)
    }

    /// The batch header plus all frozen candidates in frozen order, read from
    /// `search_batch_candidates` when the batch was frozen at schema 39+ and
    /// falling back to the blob for pre-v39 batches. Returns
    /// [`StoreError::BatchStillLoading`] when the batch is still loading, and
    /// `Ok(None)` when the batch does not exist.
    pub fn load_all_batch_candidates(
        &self,
        batch_id: &str,
    ) -> Result<Option<StoreBatchSelection>, StoreError> {
        let Some(mut batch) = self.load_batch_header(batch_id)? else {
            return Ok(None);
        };
        let items = match load_all_batch_candidate_items_connection(&self.connection, batch_id)? {
            Some(items) => items,
            None => {
                return Err(StoreError::invalid(
                    "batch selection lost its batch between reads",
                ))
            }
        };
        batch.candidates = Vec::new();
        Ok(Some(StoreBatchSelection { batch, items }))
    }
}

fn decode_candidate_json(entry: String) -> Result<SnapshotItem, StoreError> {
    let item: SnapshotItem = serde_json::from_str(&entry).map_err(StoreError::Corrupt)?;
    item.validate().map_err(StoreError::invalid)?;
    Ok(item)
}

pub(crate) fn load_all_batch_candidate_items_connection(
    connection: &rusqlite::Connection,
    batch_id: &str,
) -> Result<Option<Vec<SnapshotItem>>, StoreError> {
    let Some(kind) = candidate_rows_kind_connection(connection, batch_id)? else {
        return match crate::batch_progress::missing_batch_error(connection, batch_id) {
            StoreError::NotFound => Ok(None),
            error => Err(error),
        };
    };
    let mut blob_stmt = connection.prepare_cached(
        "SELECT candidate.value FROM search_batches b \
         JOIN json_each(b.snapshot_json, '$.candidates') candidate \
         WHERE b.batch_id = ?1 ORDER BY CAST(candidate.key AS INTEGER)",
    )?;
    let blob_items = blob_stmt
        .query_map([batch_id], |row| row.get::<_, String>(0))?
        .map(|entry| decode_candidate_json(entry?))
        .collect::<Result<Vec<_>, _>>()?;
    match kind {
        StoreCandidateKind::Rows => {
            let mut statement = connection.prepare_cached(
                "SELECT item_json FROM search_batch_candidates \
                 WHERE batch_id = ?1 ORDER BY ordinal",
            )?;
            let items = statement
                .query_map([batch_id], |row| row.get::<_, String>(0))?
                .map(|entry| decode_candidate_json(entry?))
                .collect::<Result<Vec<_>, _>>()?;
            if items != blob_items {
                return Err(StoreError::invalid(
                    "batch candidate rows disagree with the blob page slice",
                ));
            }
            Ok(Some(items))
        }
        StoreCandidateKind::Blob => Ok(Some(blob_items)),
    }
}

pub(crate) fn load_candidate_ids_connection(
    connection: &rusqlite::Connection,
    batch_id: &str,
) -> Result<Vec<String>, StoreError> {
    let kind = candidate_rows_kind_connection(connection, batch_id)?;
    let mut statement = connection.prepare(
        "SELECT json_extract(value, '$.item_id') \
         FROM search_batches b JOIN json_each(b.snapshot_json, '$.candidates') \
         WHERE b.batch_id = ?1 ORDER BY CAST(key AS INTEGER)",
    )?;
    let rows = statement.query_map([batch_id], |row| row.get::<_, String>(0))?;
    let mut blob_ids = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let item_id = row?;
        if !seen.insert(item_id.clone()) {
            return Err(StoreError::invalid(
                "workbench candidate IDs must be unique",
            ));
        }
        blob_ids.push(item_id);
    }
    if matches!(kind, Some(StoreCandidateKind::Rows)) {
        let mut row_stmt = connection.prepare_cached(
            "SELECT item_id FROM search_batch_candidates WHERE batch_id = ?1 ORDER BY ordinal",
        )?;
        let row_ids = row_stmt
            .query_map([batch_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if row_ids != blob_ids {
            return Err(StoreError::invalid(
                "batch candidate rows disagree with the blob candidate IDs",
            ));
        }
        return Ok(row_ids);
    }
    Ok(blob_ids)
}

pub(crate) fn validate_candidate_delta_connection(
    connection: &rusqlite::Connection,
    batch_id: &str,
    item_ids: &[String],
) -> Result<(), StoreError> {
    let kind = candidate_rows_kind_connection(connection, batch_id)?;
    let requested_json = serde_json::to_string(item_ids).map_err(StoreError::Corrupt)?;
    let expected = i64::try_from(item_ids.len())
        .map_err(|_| StoreError::invalid("workbench item delta is too large"))?;
    if matches!(kind, Some(StoreCandidateKind::Rows)) {
        let matched_rows: i64 = connection.query_row(
            "SELECT COUNT(*) \
             FROM search_batch_candidates c \
             JOIN json_each(?2) AS requested ON c.item_id = requested.value \
             WHERE c.batch_id = ?1",
            params![batch_id, requested_json],
            |row| row.get(0),
        )?;
        if matched_rows != expected {
            return Err(StoreError::invalid(
                "workbench item delta contains a non-candidate item",
            ));
        }
    }
    let matched_blob: i64 = connection.query_row(
        "SELECT COUNT(*) \
         FROM search_batches b \
         JOIN json_each(b.snapshot_json, '$.candidates') AS candidate \
         JOIN json_each(?2) AS requested \
           ON json_extract(candidate.value, '$.item_id') = requested.value \
         WHERE b.batch_id = ?1",
        params![batch_id, requested_json],
        |row| row.get(0),
    )?;
    if matched_blob != expected {
        return Err(StoreError::invalid(
            "workbench item delta contains a non-candidate item",
        ));
    }
    Ok(())
}

fn row_actions(
    status: Option<&DownloadItemStatus>,
    task_status: Option<&TaskLifecycleStatus>,
) -> Vec<RowTaskAction> {
    let Some(status) = status else {
        return vec![RowTaskAction::Download];
    };
    let mut actions = vec![RowTaskAction::OpenTask];
    match status {
        DownloadItemStatus::Active | DownloadItemStatus::Queued => {
            actions.push(RowTaskAction::Pause)
        }
        DownloadItemStatus::Paused | DownloadItemStatus::Interrupted => {
            actions.push(RowTaskAction::Resume)
        }
        DownloadItemStatus::Failed | DownloadItemStatus::Cancelled => {
            actions.push(RowTaskAction::Retry)
        }
        DownloadItemStatus::Completed | DownloadItemStatus::Skipped => {
            actions.push(RowTaskAction::OpenFolder)
        }
    }
    if matches!(task_status, Some(TaskLifecycleStatus::Completed))
        && !actions.contains(&RowTaskAction::OpenFolder)
    {
        actions.push(RowTaskAction::OpenFolder);
    }
    actions
}

impl TaskStore {
    /// An immutable batch remains pageable even after another search changes the
    /// single legacy Workbench slot. Rows are derived from task/runtime authority.
    pub fn load_batch_page(
        &self,
        request: &StoreBatchPageRequest,
    ) -> Result<StoreBatchPage, StoreError> {
        if request.batch_id.trim().is_empty()
            || request.page_size == 0
            || request.page_size > MAX_WORKBENCH_PAGE_SIZE
        {
            return Err(StoreError::invalid("invalid batch page request"));
        }
        // The frozen batch starts at revision 0; a nonzero revision is only
        // meaningful for the active legacy Workbench selection.
        if request.revision != 0 {
            let active: Option<i64> = self
                .connection
                .query_row(
                    "SELECT revision FROM workbench_state WHERE slot = 1 AND batch_id = ?1",
                    [&request.batch_id],
                    |row| row.get(0),
                )
                .optional()?;
            if active
                != Some(i64::try_from(request.revision).map_err(|_| StoreError::RevisionConflict)?)
            {
                return Err(StoreError::RevisionConflict);
            }
        }
        // #22 B slice 2: the frozen candidates come from the schema-39 row
        // table when the batch has rows, with the blob's `json_each` as the
        // fallback for batches frozen before v39. Same frozen order either
        // way; a partially written row set fails closed instead of serving
        // a truncated page.
        // #22 B slice 3(c): the page also carries the batch's delivery
        // state. A loading batch is served from its staged rows, and a page
        // at or beyond its arrived rows is the typed not-yet-loaded answer,
        // never an empty success. Batches never progressively published read
        // as complete.
        let (state, remote_total) =
            crate::batch_progress::progress_for_batch(&self.connection, &request.batch_id)?
                .unwrap_or((SearchBatchProgressState::Complete, None));
        let page =
            self.load_batch_candidate_page(&request.batch_id, request.page, request.page_size)?;
        let total_count = page.total_count;
        let items = page.items;
        let mut assignments: HashMap<String, (String, TaskLifecycleStatus, DownloadItemStatus)> =
            HashMap::new();
        let page_ids_json = serde_json::to_string(
            &items
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<Vec<_>>(),
        )
        .map_err(StoreError::Corrupt)?;
        // Schema-33 reverse index replaces the full task-snapshot JSON scan:
        // each row already names the source product and the task item frozen
        // from it (direct, explicit file, derived delivery or a retry-fallback
        // rebuild), so only the linked tasks' runtimes are read.
        let mut links = self.connection.prepare(
            "SELECT i.task_id, i.source_item_id, i.task_item_id FROM batch_task_items i              JOIN task_snapshots s ON s.task_id = i.task_id              JOIN task_runtime r ON r.task_id = i.task_id              LEFT JOIN task_tombstones d ON d.task_id = i.task_id              WHERE i.source_batch_id = ?1 AND i.source_item_id IN (SELECT value FROM json_each(?2))                AND d.task_id IS NULL              ORDER BY s.created_at DESC, s.task_id DESC, i.source_item_id, i.task_item_id")?;
        let mut by_task: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for link in links.query_map(params![&request.batch_id, page_ids_json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            let (task_id, source, task_item) = link?;
            match by_task.last_mut() {
                Some((current, pairs)) if *current == task_id => pairs.push((source, task_item)),
                _ => by_task.push((task_id, vec![(source, task_item)])),
            }
        }
        for (task_id, pairs) in by_task {
            let Some(task) = load_compact_task_connection(&self.connection, &task_id)? else {
                continue;
            };
            let mut statuses: HashMap<&str, Vec<DownloadItemStatus>> = HashMap::new();
            for (source, task_item) in &pairs {
                if !task.runtime.selected_item_ids.contains(task_item) {
                    continue;
                }
                statuses
                    .entry(source.as_str())
                    .or_default()
                    .push(task_page_item_status(&task.runtime, task_item, None, None));
            }
            for (source, statuses) in statuses {
                let state = aggregate_task_item_status(&statuses);
                let new_active = !matches!(
                    task.runtime.status,
                    TaskLifecycleStatus::Completed
                        | TaskLifecycleStatus::Cancelled
                        | TaskLifecycleStatus::Failed
                );
                let previous_active = assignments.get(source).is_some_and(|(_, previous, _)| {
                    !matches!(
                        previous,
                        TaskLifecycleStatus::Completed
                            | TaskLifecycleStatus::Cancelled
                            | TaskLifecycleStatus::Failed
                    )
                });
                if !assignments.contains_key(source) || (new_active && !previous_active) {
                    assignments.insert(
                        source.to_owned(),
                        (task_id.clone(), task.runtime.status.clone(), state),
                    );
                }
            }
        }
        let rows = items
            .iter()
            .map(|item| {
                let assigned = assignments.get(&item.item_id);
                let task_status = assigned.map(|(_, status, _)| status.clone());
                let item_status = assigned.map(|(_, _, status)| status.clone());
                RowTaskState {
                    item_id: item.item_id.clone(),
                    task_id: assigned.map(|(id, _, _)| id.clone()),
                    actions: row_actions(item_status.as_ref(), task_status.as_ref()),
                    task_status,
                    item_status,
                }
            })
            .collect();
        Ok(StoreBatchPage {
            batch_id: request.batch_id.clone(),
            revision: request.revision,
            page: request.page,
            page_size: request.page_size,
            total_count,
            items,
            rows,
            state,
            loaded_count: total_count,
            remote_total,
        })
    }
}
