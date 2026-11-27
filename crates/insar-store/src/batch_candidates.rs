//! #22 B slice 1 (schema 39): one row per candidate of a frozen search batch.
//!
//! The rows are written in the same transaction as the batch's
//! `search_batches.snapshot_json` blob (dual write) and repeat the blob's
//! candidates in their frozen order. Batches frozen before schema 39 have no
//! rows and are not backfilled, so readers must keep the blob fallback for
//! them.
//!
//! #22 B slice 2 adds the bounded read side of this module. Page, count and
//! membership queries answer from the rows when the batch has them and fall
//! back to the blob's `json_each` only for old batches that have no rows at
//! all. A batch whose rows exist but do not cover its whole candidate list
//! (corruption, partial write) fails closed instead of silently falling back:
//! the fallback is keyed on "this batch was frozen before v39", not on "the
//! rows are unreadable".
//!
//! While dual write lasts the blob stays the frozen authority, so a page or
//! membership answer served from the rows is also cross-checked against the
//! blob's copy of exactly the requested slice — never the whole array — and
//! fails closed when the blob disagrees. This keeps the pre-slice-2
//! fail-closed behavior for a malformed or rewritten blob item.

use insar_core::{FrozenSearchBatch, SnapshotItem};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{StoreError, TaskStore};

pub(crate) const SCHEMA_V39_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS search_batch_candidates (
    batch_id TEXT NOT NULL REFERENCES search_batches(batch_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    item_id TEXT NOT NULL CHECK(length(item_id) >= 1),
    item_json TEXT NOT NULL CHECK(length(item_json) > 1 AND json_valid(item_json)),
    PRIMARY KEY(batch_id, item_id),
    UNIQUE(batch_id, ordinal)
) STRICT;

CREATE TRIGGER IF NOT EXISTS search_batch_candidates_are_frozen
BEFORE UPDATE ON search_batch_candidates
BEGIN
    SELECT RAISE(ABORT, 'frozen search batch candidate cannot be updated');
END;
"#;

/// Writes `batch`'s candidates as rows, `ordinal` being the position in the
/// frozen candidate list. Call only in the transaction that inserts the
/// batch row itself.
pub(crate) fn insert_batch_candidates(
    connection: &Connection,
    batch: &FrozenSearchBatch,
) -> Result<(), StoreError> {
    if batch.candidates.is_empty() {
        return Ok(());
    }
    let mut statement = connection.prepare_cached(
        "INSERT INTO search_batch_candidates(batch_id, ordinal, item_id, item_json) \
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (ordinal, candidate) in batch.candidates.iter().enumerate() {
        let ordinal = i64::try_from(ordinal)
            .map_err(|_| StoreError::invalid("candidate ordinal exceeds SQLite range"))?;
        let item_json = serde_json::to_string(candidate).map_err(StoreError::Corrupt)?;
        statement.execute(params![
            batch.batch_id,
            ordinal,
            candidate.item_id,
            item_json
        ])?;
    }
    Ok(())
}

/// One bounded page of a frozen batch's candidates, in frozen order.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreBatchCandidatePage {
    pub batch_id: String,
    pub page: u64,
    pub page_size: u64,
    pub total_count: u64,
    pub items: Vec<SnapshotItem>,
}

/// A frozen batch's header plus one bounded selection of its candidates —
/// what admission and confirmation need, without materializing the whole
/// candidate array. `batch.candidates` is always empty; the selected items
/// are in `items`, in the batch's frozen order.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreBatchSelection {
    pub batch: FrozenSearchBatch,
    pub items: Vec<SnapshotItem>,
}

impl TaskStore {
    /// Whether a frozen search batch exists. Cheaper than
    /// [`Self::load_search_batch`]: no candidate array is decoded, so the
    /// plan/executor/citation existence checks do not pay for a large batch.
    pub fn search_batch_exists(&self, batch_id: &str) -> Result<bool, StoreError> {
        let exists: Option<i64> = self
            .connection
            .query_row(
                "SELECT 1 FROM search_batches WHERE batch_id = ?1",
                [batch_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    /// The frozen batch header (identity, query, AOI, stats) without its
    /// candidate array. The header is validated like a full batch load;
    /// `batch.candidates` is empty, so callers that need candidates must use
    /// the bounded page/membership/selection reads instead. `None` when the
    /// batch does not exist. A blob whose `candidates` is not an array is
    /// corruption (it must decode like a full batch), not an empty batch.
    pub fn load_batch_header(
        &self,
        batch_id: &str,
    ) -> Result<Option<FrozenSearchBatch>, StoreError> {
        // json_set replaces the array with `[]` so only the header crosses
        // the decode boundary; the CASE proves an array was there to replace.
        let row: Option<(Option<i64>, String)> = self
            .connection
            .query_row(
                "SELECT json_array_length(snapshot_json, '$.candidates'), \
                        json_set(snapshot_json, '$.candidates', json('[]')) \
                 FROM search_batches WHERE batch_id = ?1",
                [batch_id],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if row.is_none() {
            // #22 B slice 3(c): a loading batch is readable page by page but
            // has no frozen header yet; admission and selection wait for its
            // seal instead of treating it as unknown.
            if crate::batch_progress::loading_request_for_batch(&self.connection, batch_id)?
                .is_some()
            {
                return Err(StoreError::BatchStillLoading {
                    batch_id: batch_id.to_owned(),
                });
            }
        }
        row.map(|(array_length, header_json)| {
            if array_length.is_none() {
                return Err(StoreError::invalid(
                    "batch candidate array is missing or not an array",
                ));
            }
            let batch: FrozenSearchBatch =
                serde_json::from_str(&header_json).map_err(StoreError::Corrupt)?;
            batch.validate().map_err(StoreError::invalid)?;
            Ok(batch)
        })
        .transpose()
    }

    /// The batch header plus exactly the candidates named by `item_ids`, in
    /// the frozen order; ids that are not candidates of this batch are
    /// dropped. `None` when the batch does not exist. The selection is
    /// bounded by the caller (admission paths cap it at 250 today).
    pub fn load_batch_selection(
        &self,
        batch_id: &str,
        item_ids: &[String],
    ) -> Result<Option<StoreBatchSelection>, StoreError> {
        let Some(mut batch) = self.load_batch_header(batch_id)? else {
            return Ok(None);
        };
        let items = match self.load_batch_candidate_items(batch_id, item_ids)? {
            Some(items) => items,
            // The header read above already proved the batch exists, so a
            // None here means the array vanished mid-read: fail closed.
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

impl TaskStore {
    /// How the candidate rows of one batch are read: `Rows` for batches
    /// frozen at schema 39, `Blob` for older batches that have none. Rows
    /// that exist but do not match the batch's frozen candidate count are a
    /// corrupt new-format batch and fail closed in [`Self::candidate_rows_kind`].
    fn candidate_rows_kind(
        &self,
        batch_id: &str,
    ) -> Result<Option<StoreCandidateKind>, StoreError> {
        candidate_rows_kind_connection(&self.connection, batch_id)
    }

    /// One bounded page (at most `page_size` ≤ [`crate::MAX_WORKBENCH_PAGE_SIZE`])
    /// of a frozen batch's candidates, in the frozen order. Batches frozen
    /// before schema 39 have no rows and are read from their blob with the
    /// same order semantics. `NotFound` covers both an unknown batch and a
    /// page that starts beyond the last candidate; the caller never gets a
    /// silent empty page for a real batch.
    pub fn load_batch_candidate_page(
        &self,
        batch_id: &str,
        page: u64,
        page_size: u64,
    ) -> Result<StoreBatchCandidatePage, StoreError> {
        if batch_id.trim().is_empty()
            || page_size == 0
            || page_size > crate::MAX_WORKBENCH_PAGE_SIZE
        {
            return Err(StoreError::invalid("invalid candidate page request"));
        }
        // #22 B slice 3(c): a loading batch pages through its staged rows
        // (frozen item-ID order); a page at or beyond the arrived rows is the
        // typed not-yet-loaded answer, never an empty success.
        if let Some(request_id) =
            crate::batch_progress::loading_request_for_batch(&self.connection, batch_id)?
        {
            let (total_count, items) = crate::batch_progress::loading_page(
                &self.connection,
                &request_id,
                batch_id,
                page,
                page_size,
            )?;
            return Ok(StoreBatchCandidatePage {
                batch_id: batch_id.to_owned(),
                page,
                page_size,
                total_count,
                items,
            });
        }
        let kind = self
            .candidate_rows_kind(batch_id)?
            .ok_or(StoreError::NotFound)?;
        load_batch_candidate_page_connection(&self.connection, batch_id, page, page_size, kind)
    }

    /// The exact frozen candidate count of a batch, from rows or blob. `None`
    /// when the batch does not exist.
    pub fn load_batch_candidate_count(&self, batch_id: &str) -> Result<Option<u64>, StoreError> {
        let Some(kind) = self.candidate_rows_kind(batch_id)? else {
            // #22 B slice 3(c): a loading batch counts its arrived rows.
            return self.search_batch_progress(batch_id).map(|progress| {
                progress
                    .filter(|progress| {
                        progress.state == insar_core::SearchBatchProgressState::Loading
                    })
                    .map(|progress| progress.loaded_count)
            });
        };
        let count: i64 = match kind {
            StoreCandidateKind::Rows => self.connection.query_row(
                "SELECT COUNT(*) FROM search_batch_candidates WHERE batch_id = ?1",
                [batch_id],
                |row| row.get(0),
            )?,
            StoreCandidateKind::Blob => {
                let count: Option<i64> = self
                    .connection
                    .query_row(
                        "SELECT CASE WHEN json_valid(snapshot_json) \
                          THEN json_array_length(snapshot_json, '$.candidates') END \
                         FROM search_batches WHERE batch_id = ?1",
                        [batch_id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .flatten();
                count.ok_or_else(|| StoreError::invalid("batch candidate blob is invalid"))?
            }
        };
        if count < 0 {
            return Err(StoreError::invalid("batch candidate count is invalid"));
        }
        Ok(Some(u64::try_from(count).map_err(|_| {
            StoreError::invalid("batch candidate count exceeds platform range")
        })?))
    }

    /// The frozen candidates whose `item_id`s appear in `item_ids`, in the
    /// batch's frozen order; ids that are not candidates of this batch are
    /// dropped. `None` when the batch does not exist. The selection stays
    /// bounded by the caller (admission paths cap it at 250 today).
    pub fn load_batch_candidate_items(
        &self,
        batch_id: &str,
        item_ids: &[String],
    ) -> Result<Option<Vec<SnapshotItem>>, StoreError> {
        load_batch_candidate_items_connection(&self.connection, batch_id, item_ids)
    }
}

/// Bounded selection within the caller's read or write transaction.
pub(crate) fn load_batch_candidate_items_connection(
    connection: &Connection,
    batch_id: &str,
    item_ids: &[String],
) -> Result<Option<Vec<SnapshotItem>>, StoreError> {
    let Some(kind) = candidate_rows_kind_connection(connection, batch_id)? else {
        return match crate::batch_progress::missing_batch_error(connection, batch_id) {
            StoreError::NotFound => Ok(None),
            error => Err(error),
        };
    };
    if item_ids.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let requested = serde_json::to_string(item_ids).map_err(StoreError::Corrupt)?;
    match kind {
        StoreCandidateKind::Rows => {
            let mut statement = connection.prepare_cached(
                "SELECT c.item_id, c.item_json FROM search_batch_candidates c \
                     JOIN json_each(?2) AS requested ON requested.value = c.item_id \
                     WHERE c.batch_id = ?1 ORDER BY c.ordinal",
            )?;
            let found = statement
                .query_map(params![batch_id, requested], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .map(|entry| -> Result<(String, SnapshotItem), StoreError> {
                    let (item_id, json) = entry?;
                    Ok((item_id, decode_candidate_json(json)?))
                })
                .collect::<Result<Vec<_>, _>>()?;
            // Dual write keeps the blob the frozen authority, so the
            // requested membership must also decode from the blob: a
            // malformed or rewritten blob item fails closed instead of
            // being answered from the rows alone.
            cross_check_blob_membership(connection, batch_id, &requested, &found)?;
            Ok(Some(found.into_iter().map(|(_, item)| item).collect()))
        }
        StoreCandidateKind::Blob => {
            let mut statement = connection.prepare_cached(
                "SELECT json_extract(candidate.value, '$.item_id'), candidate.value \
                     FROM search_batches b \
                     JOIN json_each(b.snapshot_json, '$.candidates') candidate \
                     JOIN json_each(?2) AS requested \
                       ON requested.value = json_extract(candidate.value, '$.item_id') \
                     WHERE b.batch_id = ?1 ORDER BY CAST(candidate.key AS INTEGER)",
            )?;
            let found = statement
                .query_map(params![batch_id, requested], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .map(|entry| -> Result<(String, SnapshotItem), StoreError> {
                    let (item_id, json) = entry?;
                    Ok((item_id, decode_candidate_json(json)?))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(found.into_iter().map(|(_, item)| item).collect()))
        }
    }
}

pub(crate) enum StoreCandidateKind {
    Rows,
    Blob,
}

/// How the candidate rows of one batch are read, on an open connection (the
/// Store method or a caller inside its own transaction). `None` when the
/// batch itself does not exist. A batch whose rows exist but do not cover
/// the blob's frozen candidate count is a corrupt new-format batch and
/// fails closed rather than falling back to the blob.
pub(crate) fn candidate_rows_kind_connection(
    connection: &Connection,
    batch_id: &str,
) -> Result<Option<StoreCandidateKind>, StoreError> {
    // The blob count decides first: a batch with no row table entry is
    // either an old (pre-v39) batch — blob path — or does not exist.
    let blob_count: Option<i64> = connection
        .query_row(
            "SELECT CASE WHEN json_valid(snapshot_json) \
              THEN json_array_length(snapshot_json, '$.candidates') END \
             FROM search_batches WHERE batch_id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(blob_count) = blob_count else {
        return Ok(None); // the batch itself does not exist
    };
    if blob_count < 0 {
        return Err(StoreError::invalid("batch candidate blob is invalid"));
    }
    let row_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM search_batch_candidates WHERE batch_id = ?1",
        [batch_id],
        |row| row.get(0),
    )?;
    if row_count == 0 {
        // An empty batch legitimately has no rows (slice 1 skips the
        // insert for an empty candidate list). A non-empty blob with no
        // rows is either a pre-v39 batch (blob fallback) or a broken v39
        // batch. The discriminator is per-batch: a v39 non-empty batch
        // would have rows, but so would any other v39 batch — so a batch
        // with none while its store has some is either an old batch in a
        // mixed store (valid) or a v39 batch whose rows were lost
        // (corruption). Those two cases are indistinguishable by counts
        // alone; the fail-closed rule applies only to a batch whose own
        // rows were partially written (row_count != blob_count below).
        return Ok(Some(StoreCandidateKind::Blob));
    }
    if row_count != blob_count {
        return Err(StoreError::invalid(
            "batch candidate rows do not cover the frozen candidate count",
        ));
    }
    Ok(Some(StoreCandidateKind::Rows))
}

fn decode_candidate_json(entry: String) -> Result<SnapshotItem, StoreError> {
    let item: SnapshotItem = serde_json::from_str(&entry).map_err(StoreError::Corrupt)?;
    item.validate().map_err(StoreError::invalid)?;
    Ok(item)
}

/// #22 B slice 2, dual write: the blob is still the frozen authority, so a
/// page served from the rows must agree with the blob's copy of that same
/// slice. Decoding the page's slice of the blob (never the whole array)
/// keeps the read bounded while preserving the pre-slice-2 fail-closed
/// behavior for a malformed or rewritten blob item.
fn cross_check_blob_slice(
    connection: &Connection,
    batch_id: &str,
    items: &[SnapshotItem],
    offset_sql: i64,
    page_size_sql: i64,
) -> Result<(), StoreError> {
    let end_sql = offset_sql
        .checked_add(page_size_sql)
        .ok_or_else(|| StoreError::invalid("candidate page end overflow"))?;
    let mut statement = connection.prepare_cached(
        "SELECT CAST(candidate.key AS INTEGER), candidate.value \
         FROM search_batches b \
         JOIN json_each(b.snapshot_json, '$.candidates') candidate \
         WHERE b.batch_id = ?1 \
           AND CAST(candidate.key AS INTEGER) >= ?2 \
           AND CAST(candidate.key AS INTEGER) < ?3",
    )?;
    let blob_items = statement
        .query_map(params![batch_id, offset_sql, end_sql], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .map(|entry| {
            let (ordinal_sql, json) = entry?;
            let ordinal = u64::try_from(ordinal_sql)
                .map_err(|_| StoreError::invalid("candidate ordinal is invalid"))?;
            let item = decode_candidate_json(json)?;
            Ok((ordinal, item))
        })
        .collect::<Result<Vec<(u64, SnapshotItem)>, StoreError>>()?;
    if blob_items.len() != items.len() {
        return Err(StoreError::invalid(
            "batch candidate rows disagree with the blob page slice",
        ));
    }
    for ((blob_ordinal, blob_item), (index, row_item)) in
        blob_items.iter().zip(items.iter().enumerate())
    {
        if *blob_ordinal != offset_sql as u64 + index as u64 || blob_item != row_item {
            return Err(StoreError::invalid(
                "batch candidate rows disagree with the blob page slice",
            ));
        }
    }
    Ok(())
}

/// #22 B slice 2, dual write: like [`cross_check_blob_slice`], for a
/// membership answer. The blob's copies of exactly the requested ids are
/// decoded and must match the rows' answer, so a corrupted blob still fails
/// closed without reading the whole candidate array.
fn cross_check_blob_membership(
    connection: &Connection,
    batch_id: &str,
    requested_json: &str,
    found: &[(String, SnapshotItem)],
) -> Result<(), StoreError> {
    let mut statement = connection.prepare_cached(
        "SELECT json_extract(candidate.value, '$.item_id'), candidate.value \
         FROM search_batches b \
         JOIN json_each(b.snapshot_json, '$.candidates') candidate \
         JOIN json_each(?2) AS requested \
           ON requested.value = json_extract(candidate.value, '$.item_id') \
         WHERE b.batch_id = ?1",
    )?;
    let blob_items = statement
        .query_map(params![batch_id, requested_json], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .map(|entry| -> Result<(String, SnapshotItem), StoreError> {
            let (item_id, json) = entry?;
            let item = decode_candidate_json(json)?;
            Ok((item_id, item))
        })
        .collect::<Result<Vec<(String, SnapshotItem)>, StoreError>>()?;
    if blob_items.len() != found.len() {
        return Err(StoreError::invalid(
            "batch candidate rows disagree with the blob membership answer",
        ));
    }
    let mut blob_items = blob_items;
    blob_items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut found = found.to_vec();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    if blob_items != found {
        return Err(StoreError::invalid(
            "batch candidate rows disagree with the blob membership answer",
        ));
    }
    Ok(())
}

/// One bounded candidate page on an open connection (the Store method or a
/// caller inside its own transaction), answering from the rows for v39
/// batches and the blob's `json_each` for pre-v39 batches. The page starts
/// past the last candidate is `NotFound`; the exact boundary page (offset ==
/// total) is a valid empty tail page, matching `load_workbench_page`'s
/// original-order page semantics.
pub(crate) fn load_batch_candidate_page_connection(
    connection: &Connection,
    batch_id: &str,
    page: u64,
    page_size: u64,
    kind: StoreCandidateKind,
) -> Result<StoreBatchCandidatePage, StoreError> {
    if batch_id.trim().is_empty() || page_size == 0 || page_size > crate::MAX_WORKBENCH_PAGE_SIZE {
        return Err(StoreError::invalid("invalid candidate page request"));
    }
    let offset = page
        .checked_mul(page_size)
        .ok_or_else(|| StoreError::invalid("candidate page offset overflow"))?;
    let offset_sql = i64::try_from(offset)
        .map_err(|_| StoreError::invalid("candidate page offset exceeds SQLite range"))?;
    let page_size_sql = i64::try_from(page_size)
        .map_err(|_| StoreError::invalid("candidate page size exceeds SQLite range"))?;
    let (total, items) = match kind {
        StoreCandidateKind::Rows => {
            let total: i64 = connection.query_row(
                "SELECT COUNT(*) FROM search_batch_candidates WHERE batch_id = ?1",
                [batch_id],
                |row| row.get(0),
            )?;
            let mut statement = connection.prepare_cached(
                "SELECT item_json FROM search_batch_candidates \
                 WHERE batch_id = ?1 ORDER BY ordinal LIMIT ?2 OFFSET ?3",
            )?;
            let items = statement
                .query_map(params![batch_id, page_size_sql, offset_sql], |row| {
                    row.get::<_, String>(0)
                })?
                .map(|entry| decode_candidate_json(entry?))
                .collect::<Result<Vec<_>, _>>()?;
            // The blob is still the frozen authority while dual write lasts,
            // so the page it is serving must still agree with the blob.
            cross_check_blob_slice(connection, batch_id, &items, offset_sql, page_size_sql)?;
            (total, items)
        }
        StoreCandidateKind::Blob => {
            let total: Option<i64> = connection
                .query_row(
                    "SELECT CASE WHEN json_valid(snapshot_json) \
                      THEN json_array_length(snapshot_json, '$.candidates') END \
                     FROM search_batches WHERE batch_id = ?1",
                    [batch_id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let total =
                total.ok_or_else(|| StoreError::invalid("batch candidate blob is invalid"))?;
            let mut statement = connection.prepare_cached(
                "SELECT candidate.value FROM search_batches b \
                 JOIN json_each(b.snapshot_json, '$.candidates') candidate \
                 WHERE b.batch_id = ?1 \
                   AND CAST(candidate.key AS INTEGER) >= ?2 \
                   AND CAST(candidate.key AS INTEGER) < ?3 \
                 ORDER BY CAST(candidate.key AS INTEGER)",
            )?;
            let end_sql = offset_sql
                .checked_add(page_size_sql)
                .ok_or_else(|| StoreError::invalid("candidate page end overflow"))?;
            let items = statement
                .query_map(params![batch_id, offset_sql, end_sql], |row| {
                    row.get::<_, String>(0)
                })?
                .map(|entry| decode_candidate_json(entry?))
                .collect::<Result<Vec<_>, _>>()?;
            (total, items)
        }
    };
    if total < 0 {
        return Err(StoreError::invalid("batch candidate count is invalid"));
    }
    let total_count = u64::try_from(total)
        .map_err(|_| StoreError::invalid("batch candidate count exceeds platform range"))?;
    // A page that starts past the last candidate fails closed; the exact
    // boundary page (offset == total) is a valid empty tail page, matching
    // `load_workbench_page`'s original-order page semantics.
    if offset > total_count {
        return Err(StoreError::NotFound);
    }
    if items.len() > page_size as usize {
        return Err(StoreError::invalid("candidate page exceeds requested size"));
    }
    Ok(StoreBatchCandidatePage {
        batch_id: batch_id.to_owned(),
        page,
        page_size,
        total_count,
        items,
    })
}
