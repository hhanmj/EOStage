//! #22 B slice 3(c): progressive delivery state for search batches.
//!
//! A catalogue search becomes readable while rows are still streaming in.
//! Once the first full page has been staged, the request is *progressively
//! published*: a `loading` row here names its batch, and page reads for that
//! batch id are served from the staged rows (in the frozen item-ID order the
//! seal will use), never from a half-written `search_batches` row. The
//! frozen batch itself is written exactly once, by the bounded streaming
//! seal, when the search reaches a terminal state:
//!
//! - `complete`: the normal seal (with the catalogue cache pointer);
//! - `cancelled` / `failed`: [`TaskStore::finalize_search_batch_staging`],
//!   keeping the arrived rows under truthful, possibly-incomplete statistics
//!   and without a cache pointer;
//! - `interrupted`: [`TaskStore::interrupt_loading_search_batches`] on the
//!   next service start, for a request whose process did not survive.
//!
//! This keeps `search_batches` frozen (no rewrite, no delete), the blob and
//! candidate rows written together, and task snapshots/selection bound only
//! to a sealed batch: a loading batch is browse-only, and admission or
//! selection against it is the typed [`StoreError::BatchStillLoading`].
//! Batches frozen without progressive publication have no row here and read
//! as `complete`.

use insar_core::{FrozenSearchStats, SearchBatchProgressState, SnapshotItem};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::{StoreError, TaskStore};

pub(crate) const SCHEMA_V41_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS search_batch_progress (
    batch_id TEXT PRIMARY KEY NOT NULL
        CHECK(length(batch_id) BETWEEN 1 AND 128 AND trim(batch_id) = batch_id),
    request_id TEXT NOT NULL
        CHECK(length(request_id) BETWEEN 1 AND 128 AND trim(request_id) = request_id),
    state TEXT NOT NULL
        CHECK(state IN ('loading','complete','cancelled','failed','interrupted')),
    remote_total INTEGER CHECK(remote_total IS NULL OR remote_total >= 0),
    updated_at TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS search_batch_progress_loading_request_idx
    ON search_batch_progress(request_id) WHERE state = 'loading';
"#;

/// The staged rows a request must hold before it is progressively
/// published: one full page at the largest page size a reader may request,
/// so page 1 is complete at every page size when it first becomes readable.
/// A search that ends below it is sealed (or dropped) exactly as before
/// slice 3(c).
pub const PROGRESSIVE_PUBLICATION_ROWS: u64 = crate::MAX_WORKBENCH_PAGE_SIZE;

/// The persisted delivery row of one progressively published batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreBatchProgress {
    pub batch_id: String,
    pub request_id: String,
    pub state: SearchBatchProgressState,
    pub remote_total: Option<u64>,
    /// Rows that have arrived: the staged count while loading, the sealed
    /// candidate count afterwards.
    pub loaded_count: u64,
}

fn state_sql(state: SearchBatchProgressState) -> &'static str {
    match state {
        SearchBatchProgressState::Loading => "loading",
        SearchBatchProgressState::Complete => "complete",
        SearchBatchProgressState::Cancelled => "cancelled",
        SearchBatchProgressState::Failed => "failed",
        SearchBatchProgressState::Interrupted => "interrupted",
    }
}

fn state_from_sql(value: &str) -> Result<SearchBatchProgressState, StoreError> {
    match value {
        "loading" => Ok(SearchBatchProgressState::Loading),
        "complete" => Ok(SearchBatchProgressState::Complete),
        "cancelled" => Ok(SearchBatchProgressState::Cancelled),
        "failed" => Ok(SearchBatchProgressState::Failed),
        "interrupted" => Ok(SearchBatchProgressState::Interrupted),
        other => Err(StoreError::invalid(format!(
            "unknown search batch progress state {other}"
        ))),
    }
}

fn to_sql_count(value: u64, label: &str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::invalid(format!("{label} exceeds SQLite range")))
}

fn from_sql_count(value: i64, label: &str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::invalid(format!("{label} is invalid")))
}

struct ProgressRow {
    batch_id: String,
    request_id: String,
    state: SearchBatchProgressState,
    remote_total: Option<u64>,
}

fn decode_row(
    batch_id: String,
    request_id: String,
    state: String,
    remote_total: Option<i64>,
) -> Result<ProgressRow, StoreError> {
    Ok(ProgressRow {
        batch_id,
        request_id,
        state: state_from_sql(&state)?,
        remote_total: remote_total
            .map(|value| from_sql_count(value, "remote total"))
            .transpose()?,
    })
}

fn read_by_batch(
    connection: &Connection,
    batch_id: &str,
) -> Result<Option<ProgressRow>, StoreError> {
    connection
        .query_row(
            "SELECT batch_id, request_id, state, remote_total \
             FROM search_batch_progress WHERE batch_id = ?1",
            [batch_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .map(|(batch_id, request_id, state, total)| decode_row(batch_id, request_id, state, total))
        .transpose()
}

fn read_loading_by_request(
    connection: &Connection,
    request_id: &str,
) -> Result<Option<ProgressRow>, StoreError> {
    connection
        .query_row(
            "SELECT batch_id, request_id, state, remote_total \
             FROM search_batch_progress WHERE request_id = ?1 AND state = 'loading'",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .map(|(batch_id, request_id, state, total)| decode_row(batch_id, request_id, state, total))
        .transpose()
}

fn staged_count(connection: &Connection, request_id: &str) -> Result<u64, StoreError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM search_batch_staging_candidates WHERE request_id = ?1",
        [request_id],
        |row| row.get(0),
    )?;
    from_sql_count(count, "staged candidate count")
}

fn sealed_count(connection: &Connection, batch_id: &str) -> Result<u64, StoreError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM search_batch_candidates WHERE batch_id = ?1",
        [batch_id],
        |row| row.get(0),
    )?;
    from_sql_count(count, "sealed candidate count")
}

fn staging_exists(connection: &Connection, request_id: &str) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM search_batch_staging WHERE request_id = ?1)",
        [request_id],
        |row| row.get(0),
    )?)
}

pub(crate) fn batch_is_sealed(connection: &Connection, batch_id: &str) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM search_batches WHERE batch_id = ?1)",
        [batch_id],
        |row| row.get(0),
    )?)
}

fn project(connection: &Connection, row: ProgressRow) -> Result<StoreBatchProgress, StoreError> {
    let loaded_count = if row.state == SearchBatchProgressState::Loading {
        staged_count(connection, &row.request_id)?
    } else {
        sealed_count(connection, &row.batch_id)?
    };
    Ok(StoreBatchProgress {
        batch_id: row.batch_id,
        request_id: row.request_id,
        state: row.state,
        remote_total: row.remote_total,
        loaded_count,
    })
}

/// The delivery state and provider total of `batch_id`, `None` when the
/// batch was never progressively published.
pub(crate) fn progress_for_batch(
    connection: &Connection,
    batch_id: &str,
) -> Result<Option<(SearchBatchProgressState, Option<u64>)>, StoreError> {
    Ok(read_by_batch(connection, batch_id)?.map(|row| (row.state, row.remote_total)))
}

/// The request whose staged rows serve `batch_id` while it is loading and
/// not yet sealed; `None` for every other batch.
pub(crate) fn loading_request_for_batch(
    connection: &Connection,
    batch_id: &str,
) -> Result<Option<String>, StoreError> {
    let Some(row) = read_by_batch(connection, batch_id)? else {
        return Ok(None);
    };
    if row.state != SearchBatchProgressState::Loading || batch_is_sealed(connection, batch_id)? {
        return Ok(None);
    }
    Ok(Some(row.request_id))
}

/// `NotFound` for an unknown batch; the typed [`StoreError::BatchStillLoading`]
/// for a batch that is readable but not yet sealed.
pub(crate) fn missing_batch_error(connection: &Connection, batch_id: &str) -> StoreError {
    match loading_request_for_batch(connection, batch_id) {
        Ok(Some(_)) => StoreError::BatchStillLoading {
            batch_id: batch_id.to_owned(),
        },
        Ok(None) => StoreError::NotFound,
        Err(error) => error,
    }
}

/// One bounded page of a loading batch, read from its staged rows in the
/// frozen (item-ID) order. A page that starts at or beyond the arrived rows
/// is the typed not-yet-loaded answer, never an empty success.
pub(crate) fn loading_page(
    connection: &Connection,
    request_id: &str,
    batch_id: &str,
    page: u64,
    page_size: u64,
) -> Result<(u64, Vec<SnapshotItem>), StoreError> {
    let offset = page
        .checked_mul(page_size)
        .ok_or_else(|| StoreError::invalid("candidate page offset overflow"))?;
    let loaded_count = staged_count(connection, request_id)?;
    if offset >= loaded_count {
        return Err(StoreError::PageNotYetLoaded {
            batch_id: batch_id.to_owned(),
            loaded_count,
        });
    }
    let mut statement = connection.prepare_cached(
        "SELECT item_json FROM search_batch_staging_candidates \
         WHERE request_id = ?1 ORDER BY item_id ASC LIMIT ?2 OFFSET ?3",
    )?;
    let items = statement
        .query_map(
            params![
                request_id,
                to_sql_count(page_size, "candidate page size")?,
                to_sql_count(offset, "candidate page offset")?
            ],
            |row| row.get::<_, String>(0),
        )?
        .map(|entry| -> Result<SnapshotItem, StoreError> {
            let item: SnapshotItem = serde_json::from_str(&entry?).map_err(StoreError::Corrupt)?;
            item.validate().map_err(StoreError::invalid)?;
            Ok(item)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((loaded_count, items))
}

/// Called by every seal inside its transaction: a progressively published
/// request records its terminal state (and, when the final statistics carry
/// one, the provider total). A request that was never published is a no-op.
pub(crate) fn record_sealed_state(
    transaction: &Transaction<'_>,
    request_id: &str,
    terminal: SearchBatchProgressState,
    matched_count: Option<u64>,
) -> Result<(), StoreError> {
    if terminal == SearchBatchProgressState::Loading {
        return Err(StoreError::invalid(
            "a sealed search batch needs a terminal delivery state",
        ));
    }
    let matched_count = matched_count
        .map(|value| to_sql_count(value, "remote total"))
        .transpose()?;
    transaction.execute(
        "UPDATE search_batch_progress \
         SET state = ?2, remote_total = COALESCE(?3, remote_total), \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE request_id = ?1 AND state = 'loading'",
        params![request_id, state_sql(terminal), matched_count],
    )?;
    Ok(())
}

/// Withdraw an unsealed progressive publication with its staging (abort).
pub(crate) fn withdraw_loading(
    transaction: &Transaction<'_>,
    request_id: &str,
) -> Result<(), StoreError> {
    transaction.execute(
        "DELETE FROM search_batch_progress WHERE request_id = ?1 AND state = 'loading'",
        [request_id],
    )?;
    Ok(())
}

/// Truthful statistics for rows kept by a search that stopped before the
/// provider finished: every count is the arrived count, the provider total
/// stays what the provider reported, and the batch is possibly incomplete.
pub fn partial_search_stats(
    arrived: u64,
    remote_total: Option<u64>,
    source: &str,
) -> FrozenSearchStats {
    FrozenSearchStats {
        matched_count: remote_total,
        fetched_record_count: Some(arrived),
        mapped_record_count: Some(arrived),
        provider_returned_count: arrived,
        frozen_candidate_count: arrived,
        truncated: true,
        source_attempts: Vec::new(),
        source: source.to_owned(),
    }
}

impl TaskStore {
    /// The delivery row of a progressively published batch, with its loaded
    /// count. `None` when the batch was never progressively published (it
    /// reads as complete) or does not exist.
    pub fn search_batch_progress(
        &self,
        batch_id: &str,
    ) -> Result<Option<StoreBatchProgress>, StoreError> {
        read_by_batch(&self.connection, batch_id)?
            .map(|row| project(&self.connection, row))
            .transpose()
    }

    /// The loading publication of a request, `None` when the request is not
    /// (or no longer) loading.
    pub fn loading_search_batch_for_request(
        &self,
        request_id: &str,
    ) -> Result<Option<StoreBatchProgress>, StoreError> {
        read_loading_by_request(&self.connection, request_id)?
            .map(|row| project(&self.connection, row))
            .transpose()
    }

    /// The most recent delivery row of a request, in any state; `None` when
    /// the request never published progressively.
    pub fn latest_search_batch_progress_for_request(
        &self,
        request_id: &str,
    ) -> Result<Option<StoreBatchProgress>, StoreError> {
        self.connection
            .query_row(
                "SELECT batch_id, request_id, state, remote_total \
                 FROM search_batch_progress WHERE request_id = ?1 \
                 ORDER BY updated_at DESC, batch_id DESC LIMIT 1",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .map(|(batch_id, request_id, state, total)| {
                project(
                    &self.connection,
                    decode_row(batch_id, request_id, state, total)?,
                )
            })
            .transpose()
    }

    /// #22 B slice 3(c): make a staging request readable as a `loading`
    /// batch. Requires at least [`PROGRESSIVE_PUBLICATION_ROWS`] staged rows,
    /// so page 1 is complete when it first becomes readable. Returns the
    /// batch id. Nothing is written to `search_batches`; the batch is sealed
    /// once, at its terminal state.
    pub fn publish_search_batch_staging_progressive(
        &mut self,
        request_id: &str,
        remote_total: Option<u64>,
    ) -> Result<String, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch_id: String = transaction
            .query_row(
                "SELECT batch_id FROM search_batch_staging WHERE request_id = ?1",
                [request_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        if read_by_batch(&transaction, &batch_id)?.is_some()
            || batch_is_sealed(&transaction, &batch_id)?
        {
            return Err(StoreError::AlreadyExists);
        }
        if staged_count(&transaction, request_id)? < PROGRESSIVE_PUBLICATION_ROWS {
            return Err(StoreError::invalid(
                "progressive publication needs one full page of staged rows",
            ));
        }
        let remote_total = remote_total
            .map(|value| to_sql_count(value, "remote total"))
            .transpose()?;
        transaction.execute(
            "INSERT INTO search_batch_progress(batch_id, request_id, state, remote_total, updated_at) \
             VALUES (?1, ?2, 'loading', ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![batch_id, request_id, remote_total],
        )?;
        transaction.commit()?;
        Ok(batch_id)
    }

    /// #22 B slice 3(c): record the provider total of a loading search once
    /// the provider reports it (ASF output=count, CMR hits, STAC
    /// numberMatched, CDSE $count). Returns false when the request is not
    /// loading; an unknown total is a no-op.
    pub fn record_search_batch_remote_total(
        &mut self,
        request_id: &str,
        remote_total: Option<u64>,
    ) -> Result<bool, StoreError> {
        let Some(remote_total) = remote_total else {
            return Ok(false);
        };
        let changed = self.connection.execute(
            "UPDATE search_batch_progress \
             SET remote_total = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE request_id = ?1 AND state = 'loading' \
               AND remote_total IS NOT ?2",
            params![request_id, to_sql_count(remote_total, "remote total")?],
        )?;
        Ok(changed == 1)
    }

    /// #22 B slice 3(c): seal a loading search that stopped early —
    /// `cancelled` or `failed` — keeping its arrived rows under the given
    /// truthful statistics. The seal is the bounded streaming seal without a
    /// cache pointer (an incomplete batch never answers a later search) and
    /// activates the workbench like a completed search. Returns false when
    /// the request is no longer loading (already sealed by a racing cancel or
    /// completion), so callers may finalize idempotently.
    pub fn finalize_search_batch_staging(
        &mut self,
        request_id: &str,
        search_stats: FrozenSearchStats,
        state: SearchBatchProgressState,
    ) -> Result<bool, StoreError> {
        if !matches!(
            state,
            SearchBatchProgressState::Cancelled | SearchBatchProgressState::Failed
        ) {
            return Err(StoreError::invalid(
                "an early finalize is cancelled or failed",
            ));
        }
        if !search_stats.truncated {
            return Err(StoreError::invalid(
                "an early-stopped search batch is possibly incomplete",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read_loading_by_request(&transaction, request_id)?.is_none() {
            return Ok(false);
        }
        crate::repository::seal_staging_streaming_in_transaction(
            &transaction,
            &self.seal_read_batch_cursor,
            request_id,
            Some(search_stats),
            true,
            None,
            state,
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// #22 B slice 3(c): on service start, every publication still `loading`
    /// belongs to a search whose process did not survive. Each is sealed as
    /// `interrupted` with possibly-incomplete statistics, keeping its arrived
    /// rows; it never silently reads as complete. The workbench is not
    /// switched: a restart must not change the user's active selection. A
    /// request whose staging is gone (or cannot be sealed) is marked
    /// interrupted without a batch, so it fails closed as not found. Returns
    /// the number of batches reconciled.
    pub fn interrupt_loading_search_batches(&mut self) -> Result<usize, StoreError> {
        let loading: Vec<(String, String, Option<i64>)> = {
            let mut statement = self.connection.prepare(
                "SELECT batch_id, request_id, remote_total FROM search_batch_progress \
                 WHERE state = 'loading' ORDER BY batch_id",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut reconciled = 0_usize;
        for (batch_id, request_id, remote_total) in loading {
            let remote_total = remote_total
                .map(|value| from_sql_count(value, "remote total"))
                .transpose()?;
            let sealed = (|| -> Result<bool, StoreError> {
                let transaction = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                if !staging_exists(&transaction, &request_id)?
                    || batch_is_sealed(&transaction, &batch_id)?
                {
                    return Ok(false);
                }
                let arrived = staged_count(&transaction, &request_id)?;
                crate::repository::seal_staging_streaming_in_transaction(
                    &transaction,
                    &self.seal_read_batch_cursor,
                    &request_id,
                    Some(partial_search_stats(arrived, remote_total, "TYPED_CATALOG")),
                    false,
                    None,
                    SearchBatchProgressState::Interrupted,
                )?;
                transaction.commit()?;
                Ok(true)
            })()
            .unwrap_or(false);
            if !sealed {
                let transaction = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute(
                    "DELETE FROM search_batch_staging WHERE request_id = ?1",
                    [&request_id],
                )?;
                transaction.execute(
                    "UPDATE search_batch_progress \
                     SET state = 'interrupted', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE batch_id = ?1 AND state = 'loading'",
                    [&batch_id],
                )?;
                transaction.commit()?;
            }
            reconciled += 1;
        }
        Ok(reconciled)
    }
}
