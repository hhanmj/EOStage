use insar_core::{
    catalog_search_cache_key, decide_sar_search_cache, sar_search_cache_key, CatalogSearchQuery,
    FrozenSearchBatch, FrozenSearchStats, SarSearchCacheDecision, SarSearchQuery,
    SearchWorkbenchState, StoreSearchBatchRequest,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;

use crate::repository::save_search_batch_in_transaction;
use crate::{StoreError, TaskStore};

#[derive(Clone, Debug, PartialEq)]
pub enum StoreSarCacheDecision {
    Reuse {
        batch: FrozenSearchBatch,
    },
    Incremental {
        source_batch: FrozenSearchBatch,
        older_than: String,
        missing_results: u64,
    },
    Miss,
}

impl TaskStore {
    pub fn load_catalog_cache_batch(
        &self,
        query: &CatalogSearchQuery,
        now_epoch: u64,
        max_age_seconds: u64,
    ) -> Result<Option<FrozenSearchBatch>, StoreError> {
        let cache_key = catalog_search_cache_key(query)
            .map_err(|error| StoreError::invalid(error.user_message))?;
        let cutoff = sqlite_epoch(
            now_epoch.saturating_sub(max_age_seconds),
            "catalogue cache expiry cutoff",
        )?;
        let snapshot_json = self
            .connection
            .query_row(
                "SELECT b.snapshot_json \
                 FROM sar_search_batch_cache AS c \
                 JOIN search_batches AS b ON b.batch_id = c.source_batch_id \
                 WHERE c.cache_key = ?1 AND c.updated_at_epoch >= ?2 AND c.complete = 1 \
                 LIMIT 1",
                params![cache_key, cutoff],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(snapshot_json) = snapshot_json else {
            return Ok(None);
        };
        let batch: FrozenSearchBatch =
            serde_json::from_str(&snapshot_json).map_err(StoreError::Corrupt)?;
        batch.validate().map_err(StoreError::invalid)?;
        let frozen_query: CatalogSearchQuery =
            serde_json::from_value(Value::Object(batch.query.clone()))
                .map_err(StoreError::Corrupt)?;
        let frozen_key = catalog_search_cache_key(&frozen_query)
            .map_err(|error| StoreError::invalid(error.user_message))?;
        if frozen_key != cache_key {
            return Err(StoreError::invalid(
                "catalogue cache pointer query identity differs",
            ));
        }
        Ok(Some(batch))
    }

    pub fn seal_catalog_search_batch_staging_with_cache(
        &mut self,
        request_id: &str,
        search_stats: Option<FrozenSearchStats>,
        activate_workbench: bool,
        query: &CatalogSearchQuery,
        cache_key: &str,
        now_epoch: u64,
    ) -> Result<(), StoreError> {
        let expected_key = catalog_search_cache_key(query)
            .map_err(|error| StoreError::invalid(error.user_message))?;
        if cache_key != expected_key {
            return Err(StoreError::invalid(
                "catalogue cache key does not match the typed query",
            ));
        }
        let updated_at_epoch = sqlite_epoch(now_epoch, "catalogue cache timestamp")?;
        self.seal_search_batch_staging_with_cache_pointer(
            request_id,
            search_stats,
            activate_workbench,
            Some((cache_key, updated_at_epoch)),
        )
    }

    /// #22 B slice 3(b) review fix 2: the streaming counterpart of
    /// [`Self::seal_catalog_search_batch_staging_with_cache`]. The seal
    /// streams the staged rows into the snapshot blob without ever holding
    /// the whole candidate list in memory, and writes the same cache
    /// pointer row in the same transaction.
    pub fn seal_catalog_search_batch_staging_with_cache_streaming(
        &mut self,
        request_id: &str,
        search_stats: Option<FrozenSearchStats>,
        activate_workbench: bool,
        query: &CatalogSearchQuery,
        cache_key: &str,
        now_epoch: u64,
    ) -> Result<(), StoreError> {
        let expected_key = catalog_search_cache_key(query)
            .map_err(|error| StoreError::invalid(error.user_message))?;
        if cache_key != expected_key {
            return Err(StoreError::invalid(
                "catalogue cache key does not match the typed query",
            ));
        }
        let updated_at_epoch = sqlite_epoch(now_epoch, "catalogue cache timestamp")?;
        self.seal_search_batch_staging_streaming_with_cache_pointer(
            request_id,
            search_stats,
            activate_workbench,
            Some((cache_key, updated_at_epoch)),
        )
    }

    pub fn load_sar_cache_decision(
        &self,
        query: &SarSearchQuery,
        now_epoch: u64,
        max_age_seconds: u64,
    ) -> Result<StoreSarCacheDecision, StoreError> {
        let cache_key =
            sar_search_cache_key(query).map_err(|error| StoreError::invalid(error.user_message))?;
        let cutoff = sqlite_epoch(
            now_epoch.saturating_sub(max_age_seconds),
            "SAR cache expiry cutoff",
        )?;
        let row = self
            .connection
            .query_row(
                "SELECT c.complete, b.snapshot_json \
                 FROM sar_search_batch_cache AS c \
                 JOIN search_batches AS b ON b.batch_id = c.source_batch_id \
                 WHERE c.cache_key = ?1 AND c.updated_at_epoch >= ?2 \
                 LIMIT 1",
                params![cache_key, cutoff],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((complete, snapshot_json)) = row else {
            return Ok(StoreSarCacheDecision::Miss);
        };
        let batch: FrozenSearchBatch =
            serde_json::from_str(&snapshot_json).map_err(StoreError::Corrupt)?;
        batch.validate().map_err(StoreError::invalid)?;
        match decide_sar_search_cache(query, &batch, complete == 1)
            .map_err(|error| StoreError::invalid(error.user_message))?
        {
            SarSearchCacheDecision::Reuse => Ok(StoreSarCacheDecision::Reuse { batch }),
            SarSearchCacheDecision::Incremental {
                older_than,
                missing_results,
            } => Ok(StoreSarCacheDecision::Incremental {
                source_batch: batch,
                older_than,
                missing_results,
            }),
            SarSearchCacheDecision::Miss => Ok(StoreSarCacheDecision::Miss),
        }
    }

    pub fn save_search_batch_with_sar_cache(
        &mut self,
        request: &StoreSearchBatchRequest,
        cache_key: &str,
        now_epoch: u64,
        complete: bool,
    ) -> Result<(), StoreError> {
        let frozen_query: SarSearchQuery =
            serde_json::from_value(Value::Object(request.batch.query.clone()))
                .map_err(StoreError::Corrupt)?;
        let expected_key = sar_search_cache_key(&frozen_query)
            .map_err(|error| StoreError::invalid(error.user_message))?;
        if cache_key != expected_key {
            return Err(StoreError::invalid(
                "SAR cache key does not match the frozen typed query",
            ));
        }
        if cache_key.len() != 64
            || !cache_key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(StoreError::invalid(
                "SAR cache key is not canonical SHA-256",
            ));
        }
        let updated_at_epoch = sqlite_epoch(now_epoch, "SAR cache timestamp")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_search_batch_in_transaction(&transaction, request)?;
        transaction.execute(
            "INSERT INTO sar_search_batch_cache(\
                 cache_key, source_batch_id, updated_at_epoch, complete\
             ) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(cache_key) DO UPDATE SET \
                 source_batch_id = excluded.source_batch_id, \
                 updated_at_epoch = excluded.updated_at_epoch, \
                 complete = excluded.complete",
            params![
                cache_key,
                request.batch.batch_id,
                updated_at_epoch,
                i64::from(complete)
            ],
        )?;
        crate::cache_maintenance::prune_disposable_search_batches_in_tx(
            &transaction,
            Some(&request.batch.batch_id),
            Some(now_epoch),
            crate::cache_maintenance::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            crate::cache_maintenance::SEARCH_CACHE_MAX_ENTRIES,
            crate::cache_maintenance::SEARCH_CACHE_MAX_BYTES,
            false,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn update_active_sar_matched_count(
        &mut self,
        batch_id: &str,
        expected_workbench_revision: u64,
        expected_query_fingerprint: &str,
        matched_count: u64,
    ) -> Result<bool, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active = transaction
            .query_row(
                "SELECT w.batch_id, w.revision, w.state_json, b.snapshot_json \
                 FROM workbench_state AS w \
                 JOIN search_batches AS b ON b.batch_id = w.batch_id \
                 WHERE w.slot = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((active_batch_id, revision, state_json, batch_json)) = active else {
            return Ok(false);
        };
        let revision = u64::try_from(revision)
            .map_err(|_| StoreError::invalid("workbench revision is invalid"))?;
        if active_batch_id != batch_id || revision != expected_workbench_revision {
            return Ok(false);
        }
        let state: SearchWorkbenchState =
            serde_json::from_str(&state_json).map_err(StoreError::Corrupt)?;
        state.validate().map_err(StoreError::invalid)?;
        if state.batch_id != active_batch_id || state.revision != revision {
            return Err(StoreError::invalid("workbench state envelope differs"));
        }
        let batch: FrozenSearchBatch =
            serde_json::from_str(&batch_json).map_err(StoreError::Corrupt)?;
        batch.validate().map_err(StoreError::invalid)?;
        let frozen_query: SarSearchQuery =
            serde_json::from_value(Value::Object(batch.query.clone()))
                .map_err(StoreError::Corrupt)?;
        let actual_fingerprint = sar_search_cache_key(&frozen_query)
            .map_err(|error| StoreError::invalid(error.user_message))?;
        if actual_fingerprint != expected_query_fingerprint {
            return Ok(false);
        }
        if batch.search_stats.is_none() {
            return Ok(false);
        }
        let matched_count = i64::try_from(matched_count)
            .map_err(|_| StoreError::invalid("SAR matched count exceeds SQLite range"))?;
        let observed_revision = i64::try_from(expected_workbench_revision)
            .map_err(|_| StoreError::invalid("workbench revision exceeds SQLite range"))?;
        let changed = transaction.execute(
            "INSERT INTO sar_search_batch_count( \
                 batch_id, query_fingerprint, observed_workbench_revision, matched_count, updated_at\
             ) VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
             ON CONFLICT(batch_id) DO UPDATE SET \
                 query_fingerprint = excluded.query_fingerprint, \
                 observed_workbench_revision = excluded.observed_workbench_revision, \
                 matched_count = excluded.matched_count, \
                 updated_at = excluded.updated_at",
            params![
                batch_id,
                expected_query_fingerprint,
                observed_revision,
                matched_count
            ],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        transaction.commit()?;
        Ok(true)
    }
}

fn sqlite_epoch(value: u64, label: &str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::invalid(format!("{label} exceeds SQLite range")))
}
