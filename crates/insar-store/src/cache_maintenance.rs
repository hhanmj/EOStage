//! Bounded cache eviction, inspection, and safe clearing for disposable SQLite caches.
//!
//! Disposable caches (search result caches, unreferenced historical search batches,
//! abandoned staging rows, and cached administrative boundary geometries) are kept
//! strictly separate from user-owned data (active workbench state, task snapshots,
//! download history, source views, bound AOIs, and credentials).

use rusqlite::{params, Transaction, TransactionBehavior};

use crate::{StoreError, TaskStore};

/// Default TTL (24 hours) for cached SAR/catalog search entries.
pub const SEARCH_CACHE_DEFAULT_TTL_SECONDS: u64 = 24 * 60 * 60;
/// Maximum number of unreferenced disposable search batches retained in SQLite.
pub const SEARCH_CACHE_MAX_ENTRIES: usize = 64;
/// Maximum byte budget (128 MiB) for disposable search cache + unreferenced search batches.
pub const SEARCH_CACHE_MAX_BYTES: u64 = 128 * 1024 * 1024;

/// SQL predicate identifying `search_batches` rows that are NOT referenced by any
/// active workbench slot, task snapshot, source view, task rebind, task head, or
/// batch-to-task membership row.
const UNREFERENCED_SEARCH_BATCH_FILTER: &str = r#"
    b.batch_id NOT IN (SELECT batch_id FROM workbench_state WHERE batch_id IS NOT NULL)
    AND b.batch_id NOT IN (SELECT batch_id FROM task_snapshots WHERE batch_id IS NOT NULL)
    AND b.batch_id NOT IN (SELECT batch_id FROM source_views WHERE batch_id IS NOT NULL)
    AND b.batch_id NOT IN (
        SELECT p.batch_id
        FROM search_batch_progress AS p
        JOIN source_views AS sv ON sv.pending_request_id = p.request_id
        WHERE p.batch_id IS NOT NULL
    )
    AND b.batch_id NOT IN (SELECT source_batch_id FROM task_item_rebinds WHERE source_batch_id IS NOT NULL)
    AND b.batch_id NOT IN (SELECT source_batch_id FROM task_snapshot_heads WHERE source_batch_id IS NOT NULL)
    AND b.batch_id NOT IN (SELECT offered_batch_id FROM task_snapshot_heads WHERE offered_batch_id IS NOT NULL)
    AND b.batch_id NOT IN (SELECT DISTINCT source_batch_id FROM batch_task_items WHERE source_batch_id IS NOT NULL)
    AND b.batch_id NOT IN (SELECT batch_id FROM search_batch_progress WHERE state = 'loading' AND batch_id IS NOT NULL)
"#;

/// Read-only accounting of disposable SQLite cache categories.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreDisposableCacheStats {
    pub search_cache_bytes: u64,
    pub search_cache_entries: u64,
    pub admin_boundary_cache_bytes: u64,
    pub admin_boundary_cache_entries: u64,
    pub sqlite_freelist_bytes: u64,
}

/// Summary of rows and payload bytes removed by a cache clear or pruning operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreCacheClearOutcome {
    pub removed_entries: u64,
    pub reclaimed_bytes: u64,
}

pub(crate) fn prune_disposable_search_batches_in_tx(
    transaction: &Transaction<'_>,
    protected_batch_id: Option<&str>,
    now_epoch: Option<u64>,
    ttl_seconds: u64,
    max_entries: usize,
    max_bytes: u64,
    cleanup_all_orphan_staging: bool,
) -> Result<StoreCacheClearOutcome, StoreError> {
    let mut removed_entries: u64 = 0;
    let mut reclaimed_bytes: u64 = 0;

    // 1. Clean abandoned staging rows that are no longer actively loading or pending.
    let (staging_where, deleted_staging_bytes): (&str, i64) = if cleanup_all_orphan_staging {
        let sql = "request_id NOT IN (
                       SELECT pending_request_id FROM source_views WHERE pending_request_id IS NOT NULL
                   )
                   AND request_id NOT IN (
                       SELECT request_id FROM search_batch_progress WHERE state = 'loading'
                   )";
        let header_bytes: i64 = transaction.query_row(
            &format!(
                "SELECT COALESCE(SUM(LENGTH(CAST(batch_header_json AS BLOB))
                                   + LENGTH(CAST(workbench_json AS BLOB))), 0)
                 FROM search_batch_staging WHERE {sql}"
            ),
            [],
            |row| row.get(0),
        )?;
        let cand_bytes: i64 = transaction.query_row(
            &format!(
                "SELECT COALESCE(SUM(LENGTH(CAST(item_json AS BLOB))), 0)
                 FROM search_batch_staging_candidates
                 WHERE request_id IN (SELECT request_id FROM search_batch_staging WHERE {sql})"
            ),
            [],
            |row| row.get(0),
        )?;
        (sql, header_bytes.saturating_add(cand_bytes))
    } else {
        let sql = "request_id NOT IN (
                       SELECT pending_request_id FROM source_views WHERE pending_request_id IS NOT NULL
                   )
                   AND request_id IN (
                       SELECT request_id FROM search_batch_progress
                       WHERE state IN ('interrupted', 'failed', 'cancelled', 'complete')
                   )";
        let header_bytes: i64 = transaction.query_row(
            &format!(
                "SELECT COALESCE(SUM(LENGTH(CAST(batch_header_json AS BLOB))
                                   + LENGTH(CAST(workbench_json AS BLOB))), 0)
                 FROM search_batch_staging WHERE {sql}"
            ),
            [],
            |row| row.get(0),
        )?;
        let cand_bytes: i64 = transaction.query_row(
            &format!(
                "SELECT COALESCE(SUM(LENGTH(CAST(item_json AS BLOB))), 0)
                 FROM search_batch_staging_candidates
                 WHERE request_id IN (SELECT request_id FROM search_batch_staging WHERE {sql})"
            ),
            [],
            |row| row.get(0),
        )?;
        (sql, header_bytes.saturating_add(cand_bytes))
    };
    let deleted_staging = transaction.execute(
        &format!("DELETE FROM search_batch_staging WHERE {staging_where}"),
        [],
    )?;
    if deleted_staging > 0 {
        removed_entries = removed_entries.saturating_add(deleted_staging as u64);
        reclaimed_bytes =
            reclaimed_bytes.saturating_add(u64::try_from(deleted_staging_bytes).unwrap_or(0));
    }

    // 2. Prune expired `sar_search_batch_cache` pointers and 24h-expired unreferenced `search_batches`.
    if let Some(now) = now_epoch.filter(|&n| n > ttl_seconds) {
        if let Ok(cutoff) = i64::try_from(now.saturating_sub(ttl_seconds)) {
            let deleted = transaction.execute(
                "DELETE FROM sar_search_batch_cache WHERE updated_at_epoch < ?1",
                [cutoff],
            )?;
            removed_entries = removed_entries.saturating_add(deleted as u64);

            let expired_sql = format!(
                "SELECT b.batch_id,
                        LENGTH(CAST(b.snapshot_json AS BLOB))
                          + COALESCE((SELECT SUM(LENGTH(CAST(c.item_json AS BLOB)))
                                      FROM search_batch_candidates AS c
                                      WHERE c.batch_id = b.batch_id), 0) AS total_bytes
                 FROM search_batches AS b
                 WHERE {UNREFERENCED_SEARCH_BATCH_FILTER}
                   AND (?1 IS NULL OR b.batch_id != ?1)
                   AND strftime('%s', b.created_at) IS NOT NULL
                   AND CAST(strftime('%s', b.created_at) AS INTEGER) < ?2
                   AND NOT EXISTS(
                       SELECT 1 FROM sar_search_batch_cache AS sc
                       WHERE sc.source_batch_id = b.batch_id
                         AND sc.updated_at_epoch >= ?2
                   )"
            );
            let expired_rows: Vec<(String, u64)> = {
                let mut stmt = transaction.prepare(&expired_sql)?;
                let mapped = stmt.query_map(params![protected_batch_id, cutoff], |row| {
                    let id: String = row.get(0)?;
                    let bytes: i64 = row.get(1)?;
                    Ok((id, u64::try_from(bytes).unwrap_or(0)))
                })?;
                mapped.collect::<Result<Vec<_>, _>>()?
            };
            for (batch_id, bytes) in expired_rows {
                let deleted = transaction.execute(
                    "DELETE FROM search_batches WHERE batch_id = ?1",
                    params![batch_id],
                )?;
                if deleted > 0 {
                    removed_entries = removed_entries.saturating_add(deleted as u64);
                    reclaimed_bytes = reclaimed_bytes.saturating_add(bytes);
                }
            }
        }
    }

    // 3. Account for `asf_search_cache` in the shared search-cache entry & byte budget.
    let (asf_entries_i64, asf_bytes_i64): (i64, i64) = transaction.query_row(
        "SELECT COUNT(*), COALESCE(SUM(LENGTH(CAST(entry_json AS BLOB))), 0)
         FROM asf_search_cache",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut kept_entries: usize = usize::try_from(asf_entries_i64).unwrap_or(0);
    let mut kept_bytes: u64 = u64::try_from(asf_bytes_i64).unwrap_or(0);

    // 4. Load remaining unreferenced disposable search batches ordered newest-first (LRU order).
    let query = format!(
        "SELECT b.batch_id,
                LENGTH(CAST(b.snapshot_json AS BLOB))
                  + COALESCE((SELECT SUM(LENGTH(CAST(c.item_json AS BLOB)))
                              FROM search_batch_candidates AS c
                              WHERE c.batch_id = b.batch_id), 0) AS total_bytes
         FROM search_batches AS b
         WHERE {UNREFERENCED_SEARCH_BATCH_FILTER}
         ORDER BY COALESCE((SELECT MAX(sc.updated_at_epoch)
                            FROM sar_search_batch_cache AS sc
                            WHERE sc.source_batch_id = b.batch_id), 0) DESC,
                  b.created_at DESC,
                  b.batch_id DESC"
    );
    let rows: Vec<(String, u64)> = {
        let mut stmt = transaction.prepare(&query)?;
        let mapped = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let bytes: i64 = row.get(1)?;
            Ok((id, u64::try_from(bytes).unwrap_or(0)))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };

    let mut to_evict: Vec<(String, u64)> = Vec::new();
    for (batch_id, bytes) in rows {
        if protected_batch_id == Some(batch_id.as_str()) {
            kept_entries = kept_entries.saturating_add(1);
            kept_bytes = kept_bytes.saturating_add(bytes);
            continue;
        }
        let next_entries = kept_entries.saturating_add(1);
        let next_bytes = kept_bytes.saturating_add(bytes);
        if next_entries <= max_entries && next_bytes <= max_bytes {
            kept_entries = next_entries;
            kept_bytes = next_bytes;
        } else {
            to_evict.push((batch_id, bytes));
        }
    }

    for (batch_id, bytes) in to_evict {
        let deleted = transaction.execute(
            "DELETE FROM search_batches WHERE batch_id = ?1",
            params![batch_id],
        )?;
        if deleted > 0 {
            removed_entries = removed_entries.saturating_add(deleted as u64);
            reclaimed_bytes = reclaimed_bytes.saturating_add(bytes);
        }
    }

    Ok(StoreCacheClearOutcome {
        removed_entries,
        reclaimed_bytes,
    })
}

impl TaskStore {
    pub const SEARCH_CACHE_DEFAULT_TTL_SECONDS: u64 = SEARCH_CACHE_DEFAULT_TTL_SECONDS;
    pub const SEARCH_CACHE_MAX_ENTRIES: usize = SEARCH_CACHE_MAX_ENTRIES;
    pub const SEARCH_CACHE_MAX_BYTES: u64 = SEARCH_CACHE_MAX_BYTES;
    pub const ADMIN_BOUNDARY_CACHE_DEFAULT_TTL_SECONDS: u64 =
        crate::ADMIN_BOUNDARY_CACHE_DEFAULT_TTL_SECONDS;
    pub const ADMIN_BOUNDARY_CACHE_MAX_ENTRIES: usize = crate::ADMIN_BOUNDARY_CACHE_MAX_ENTRIES;
    pub const ADMIN_BOUNDARY_CACHE_MAX_BYTES: u64 = crate::ADMIN_BOUNDARY_CACHE_MAX_BYTES;

    /// Inspects current size and entry counts for all disposable SQLite cache tables.
    pub fn inspect_disposable_cache_stats(&self) -> Result<StoreDisposableCacheStats, StoreError> {
        let (asf_entries, asf_bytes): (i64, i64) = self.connection.query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(CAST(entry_json AS BLOB))), 0)
             FROM asf_search_cache",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let unref_batch_sql = format!(
            "SELECT COUNT(*),
                    COALESCE(SUM(LENGTH(CAST(b.snapshot_json AS BLOB))), 0)
             FROM search_batches AS b
             WHERE {UNREFERENCED_SEARCH_BATCH_FILTER}"
        );
        let (unref_batches, unref_batch_bytes): (i64, i64) =
            self.connection
                .query_row(&unref_batch_sql, [], |row| Ok((row.get(0)?, row.get(1)?)))?;

        let unref_cand_sql = format!(
            "SELECT COALESCE(SUM(LENGTH(CAST(c.item_json AS BLOB))), 0)
             FROM search_batch_candidates AS c
             WHERE c.batch_id IN (
                 SELECT b.batch_id FROM search_batches AS b
                 WHERE {UNREFERENCED_SEARCH_BATCH_FILTER}
             )"
        );
        let unref_cand_bytes: i64 = self
            .connection
            .query_row(&unref_cand_sql, [], |row| row.get(0))?;

        let (stale_staging_entries, stale_staging_bytes): (i64, i64) =
            self.connection.query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(LENGTH(CAST(batch_header_json AS BLOB))
                                   + LENGTH(CAST(workbench_json AS BLOB))), 0)
                 FROM search_batch_staging
                 WHERE request_id NOT IN (
                     SELECT pending_request_id FROM source_views WHERE pending_request_id IS NOT NULL
                 )
                 AND request_id NOT IN (
                     SELECT request_id FROM search_batch_progress WHERE state = 'loading'
                 )",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
        let stale_staging_cand_bytes: i64 = self.connection.query_row(
            "SELECT COALESCE(SUM(LENGTH(CAST(item_json AS BLOB))), 0)
             FROM search_batch_staging_candidates
             WHERE request_id NOT IN (
                 SELECT pending_request_id FROM source_views WHERE pending_request_id IS NOT NULL
             )
             AND request_id NOT IN (
                 SELECT request_id FROM search_batch_progress WHERE state = 'loading'
             )",
            [],
            |row| row.get(0),
        )?;

        let (sar_ptr_entries, sar_ptr_bytes): (i64, i64) = self.connection.query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(CAST(cache_key AS BLOB)) + LENGTH(CAST(source_batch_id AS BLOB))), 0)
             FROM sar_search_batch_cache",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let (admin_entries, admin_bytes): (i64, i64) = self.connection.query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(CAST(geometry_json AS BLOB))), 0)
             FROM administrative_boundary_cache",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let freelist_count: i64 = self
            .connection
            .pragma_query_value(None, "freelist_count", |row| row.get(0))
            .unwrap_or(0);
        let page_size: i64 = self
            .connection
            .pragma_query_value(None, "page_size", |row| row.get(0))
            .unwrap_or(4096);

        let search_cache_bytes = u64::try_from(asf_bytes)
            .unwrap_or(0)
            .saturating_add(u64::try_from(unref_batch_bytes).unwrap_or(0))
            .saturating_add(u64::try_from(unref_cand_bytes).unwrap_or(0))
            .saturating_add(u64::try_from(stale_staging_bytes).unwrap_or(0))
            .saturating_add(u64::try_from(stale_staging_cand_bytes).unwrap_or(0))
            .saturating_add(u64::try_from(sar_ptr_bytes).unwrap_or(0));

        // Count user-meaningful cached search items (unreferenced batches + legacy ASF entries
        // + abandoned staging entries; fall back to pointer rows if batches are all pinned).
        let search_cache_entries = u64::try_from(asf_entries)
            .unwrap_or(0)
            .saturating_add(u64::try_from(unref_batches).unwrap_or(0))
            .saturating_add(u64::try_from(stale_staging_entries).unwrap_or(0))
            .max(u64::try_from(sar_ptr_entries).unwrap_or(0));

        Ok(StoreDisposableCacheStats {
            search_cache_bytes,
            search_cache_entries,
            admin_boundary_cache_bytes: u64::try_from(admin_bytes).unwrap_or(0),
            admin_boundary_cache_entries: u64::try_from(admin_entries).unwrap_or(0),
            sqlite_freelist_bytes: u64::try_from(freelist_count)
                .unwrap_or(0)
                .saturating_mul(u64::try_from(page_size).unwrap_or(4096)),
        })
    }

    /// Prunes expired and LRU-excess disposable search cache rows to stay within budget.
    pub fn prune_disposable_search_cache_bounds(
        &mut self,
        now_epoch: u64,
        ttl_seconds: u64,
        max_entries: usize,
        max_bytes: u64,
    ) -> Result<StoreCacheClearOutcome, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = prune_disposable_search_batches_in_tx(
            &transaction,
            None,
            Some(now_epoch),
            ttl_seconds,
            max_entries,
            max_bytes,
            true,
        )?;
        transaction.commit()?;
        Ok(outcome)
    }

    /// Runs bounded automatic cache eviction across legacy ASF search cache,
    /// unreferenced search batches / abandoned staging rows, and cached
    /// administrative boundaries without touching user-owned records.
    pub fn prune_all_disposable_caches(
        &mut self,
        now_epoch: u64,
        admin_cutoff_iso: &str,
    ) -> Result<StoreCacheClearOutcome, StoreError> {
        let before = self.inspect_disposable_cache_stats()?;
        let asf_removed = self.prune_asf_cache_bounds(
            now_epoch,
            crate::ASF_SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            crate::ASF_SEARCH_CACHE_MAX_ENTRIES,
            crate::ASF_SEARCH_CACHE_MAX_BYTES,
        )? as u64;
        let search_outcome = self.prune_disposable_search_cache_bounds(
            now_epoch,
            SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            SEARCH_CACHE_MAX_ENTRIES,
            SEARCH_CACHE_MAX_BYTES,
        )?;
        let admin_removed = self.prune_administrative_boundary_cache_bounds(
            Some(admin_cutoff_iso),
            crate::ADMIN_BOUNDARY_CACHE_MAX_ENTRIES,
            crate::ADMIN_BOUNDARY_CACHE_MAX_BYTES,
        )? as u64;
        let after = self.inspect_disposable_cache_stats()?;
        let before_total = before
            .search_cache_bytes
            .saturating_add(before.admin_boundary_cache_bytes);
        let after_total = after
            .search_cache_bytes
            .saturating_add(after.admin_boundary_cache_bytes);
        Ok(StoreCacheClearOutcome {
            removed_entries: asf_removed
                .saturating_add(search_outcome.removed_entries)
                .saturating_add(admin_removed),
            reclaimed_bytes: before_total
                .saturating_sub(after_total)
                .max(search_outcome.reclaimed_bytes),
        })
    }

    /// Clears all disposable search caches (`asf_search_cache`, `sar_search_batch_cache`,
    /// unreferenced `search_batches` + cascading candidates, and abandoned `search_batch_staging`)
    /// without touching active workbench state, tasks, history, or source views.
    pub fn clear_disposable_search_cache(&mut self) -> Result<StoreCacheClearOutcome, StoreError> {
        let before = self.inspect_disposable_cache_stats()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let asf_deleted = transaction.execute("DELETE FROM asf_search_cache", [])? as u64;
        let ptr_deleted = transaction.execute("DELETE FROM sar_search_batch_cache", [])? as u64;
        let delete_batches_sql = format!(
            "DELETE FROM search_batches
             WHERE batch_id IN (
                 SELECT b.batch_id FROM search_batches AS b
                 WHERE {UNREFERENCED_SEARCH_BATCH_FILTER}
             )"
        );
        let batches_deleted = transaction.execute(&delete_batches_sql, [])? as u64;
        let staging_deleted = transaction.execute(
            "DELETE FROM search_batch_staging
             WHERE request_id NOT IN (
                 SELECT pending_request_id FROM source_views WHERE pending_request_id IS NOT NULL
             )
             AND request_id NOT IN (
                 SELECT request_id FROM search_batch_progress WHERE state = 'loading'
             )",
            [],
        )? as u64;
        transaction.commit()?;
        let after = self.inspect_disposable_cache_stats()?;
        Ok(StoreCacheClearOutcome {
            removed_entries: asf_deleted
                .saturating_add(batches_deleted)
                .saturating_add(staging_deleted)
                .max(ptr_deleted),
            reclaimed_bytes: before
                .search_cache_bytes
                .saturating_sub(after.search_cache_bytes),
        })
    }

    /// Clears all cached administrative boundary geometries (`administrative_boundary_cache`)
    /// without touching user-bound AOIs (`region_aoi_state`, `standalone_aoi_state`).
    pub fn clear_administrative_boundary_cache(
        &mut self,
    ) -> Result<StoreCacheClearOutcome, StoreError> {
        let before = self.inspect_disposable_cache_stats()?;
        let deleted = self
            .connection
            .execute("DELETE FROM administrative_boundary_cache", [])? as u64;
        Ok(StoreCacheClearOutcome {
            removed_entries: deleted,
            reclaimed_bytes: before.admin_boundary_cache_bytes,
        })
    }

    /// Checkpoints WAL and optionally runs `VACUUM` so reclaimed space is returned to the OS.
    pub fn reclaim_sqlite_space(&mut self, run_vacuum: bool) -> Result<bool, StoreError> {
        let _ = self
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        if !run_vacuum {
            return Ok(false);
        }
        match self.connection.execute_batch("VACUUM;") {
            Ok(()) => {
                let _ = self
                    .connection
                    .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}
