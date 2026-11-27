use std::collections::BTreeMap;

use insar_core::{AsfCacheEntry, AsfSceneMetadata};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

use crate::{StoreError, TaskStore};

/// Default TTL (24 hours) for legacy `asf_search_cache` entries.
pub const ASF_SEARCH_CACHE_DEFAULT_TTL_SECONDS: u64 = 24 * 60 * 60;
/// Maximum cached entries retained in `asf_search_cache`.
pub const ASF_SEARCH_CACHE_MAX_ENTRIES: usize = 64;
/// Maximum total JSON payload budget (32 MiB) retained in `asf_search_cache`.
pub const ASF_SEARCH_CACHE_MAX_BYTES: u64 = 32 * 1024 * 1024;

impl TaskStore {
    pub fn upsert_asf_cache_entry(
        &mut self,
        entry: &AsfCacheEntry,
        now_epoch: u64,
    ) -> Result<(), StoreError> {
        entry.validate().map_err(StoreError::invalid)?;
        let now_epoch_i64 = sqlite_epoch(now_epoch, "ASF cache timestamp")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_json = transaction
            .query_row(
                "SELECT entry_json FROM asf_search_cache WHERE cache_key = ?1",
                [&entry.cache_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let merged = match existing_json {
            Some(value) => {
                let existing: AsfCacheEntry =
                    serde_json::from_str(&value).map_err(StoreError::Corrupt)?;
                existing.validate().map_err(StoreError::invalid)?;
                merge_entries(existing, entry.clone())?
            }
            None => entry.clone(),
        };
        let entry_json = serde_json::to_string(&merged).map_err(StoreError::Corrupt)?;
        transaction.execute(
            "INSERT INTO asf_search_cache(\
                 cache_key, created_at_epoch, updated_at_epoch, entry_json\
             ) VALUES (?1, ?2, ?2, ?3) \
             ON CONFLICT(cache_key) DO UPDATE SET \
                 updated_at_epoch = excluded.updated_at_epoch, \
                 entry_json = excluded.entry_json",
            params![merged.cache_key, now_epoch_i64, entry_json],
        )?;
        prune_asf_cache_bounds_in_tx(
            &transaction,
            Some(&merged.cache_key),
            now_epoch,
            ASF_SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            ASF_SEARCH_CACHE_MAX_ENTRIES,
            ASF_SEARCH_CACHE_MAX_BYTES,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn prune_asf_cache_bounds(
        &mut self,
        now_epoch: u64,
        ttl_seconds: u64,
        max_entries: usize,
        max_bytes: u64,
    ) -> Result<usize, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = prune_asf_cache_bounds_in_tx(
            &transaction,
            None,
            now_epoch,
            ttl_seconds,
            max_entries,
            max_bytes,
        )?;
        transaction.commit()?;
        Ok(removed)
    }

    pub fn load_fresh_asf_cache_entries(
        &self,
        now_epoch: u64,
        max_age_seconds: u64,
    ) -> Result<Vec<AsfCacheEntry>, StoreError> {
        let cutoff = sqlite_epoch(
            now_epoch.saturating_sub(max_age_seconds),
            "ASF cache expiry cutoff",
        )?;
        let mut statement = self.connection.prepare(
            "SELECT entry_json FROM asf_search_cache \
             WHERE updated_at_epoch >= ?1 \
             ORDER BY updated_at_epoch DESC, cache_key ASC",
        )?;
        let rows = statement.query_map([cutoff], |row| row.get::<_, String>(0))?;
        let mut entries = Vec::new();
        for row in rows {
            let value = row?;
            let entry: AsfCacheEntry = serde_json::from_str(&value).map_err(StoreError::Corrupt)?;
            entry.validate().map_err(StoreError::invalid)?;
            entries.push(entry);
        }
        Ok(entries)
    }

    pub fn prune_expired_asf_cache_entries(
        &mut self,
        now_epoch: u64,
        max_age_seconds: u64,
    ) -> Result<usize, StoreError> {
        let cutoff = sqlite_epoch(
            now_epoch.saturating_sub(max_age_seconds),
            "ASF cache expiry cutoff",
        )?;
        self.connection
            .execute(
                "DELETE FROM asf_search_cache WHERE updated_at_epoch < ?1",
                [cutoff],
            )
            .map_err(StoreError::from)
    }
}

fn merge_entries(
    existing: AsfCacheEntry,
    incoming: AsfCacheEntry,
) -> Result<AsfCacheEntry, StoreError> {
    if existing.cache_key != incoming.cache_key {
        return Err(StoreError::invalid("ASF cache keys differ during merge"));
    }
    let mut scenes = existing
        .scenes
        .into_iter()
        .map(|scene| (scene.scene_id.clone(), scene))
        .collect::<BTreeMap<String, AsfSceneMetadata>>();
    for scene in incoming.scenes {
        scenes.insert(scene.scene_id.clone(), scene);
    }
    let mut scenes = scenes.into_values().collect::<Vec<_>>();
    scenes.sort_by(|left, right| {
        right
            .acquisition_datetime
            .cmp(&left.acquisition_datetime)
            .then_with(|| left.scene_id.cmp(&right.scene_id))
    });
    let total_count = incoming.total_count.or(existing.total_count);
    let complete = incoming.complete
        || existing.complete
        || total_count.is_some_and(|total| scenes.len() as u64 >= total);
    let merged = AsfCacheEntry {
        cache_key: incoming.cache_key,
        query: incoming.query,
        scenes,
        complete,
        total_count,
    };
    merged.validate().map_err(StoreError::invalid)?;
    Ok(merged)
}

fn prune_asf_cache_bounds_in_tx(
    transaction: &Transaction<'_>,
    protected_key: Option<&str>,
    now_epoch: u64,
    ttl_seconds: u64,
    max_entries: usize,
    max_bytes: u64,
) -> Result<usize, StoreError> {
    let mut removed: usize = 0;
    if now_epoch > ttl_seconds {
        let cutoff = sqlite_epoch(
            now_epoch.saturating_sub(ttl_seconds),
            "ASF cache expiry cutoff",
        )?;
        removed = removed.saturating_add(transaction.execute(
            "DELETE FROM asf_search_cache WHERE updated_at_epoch < ?1 AND (?2 IS NULL OR cache_key != ?2)",
            params![cutoff, protected_key],
        )?);
    }

    let rows: Vec<(String, u64)> = {
        let mut stmt = transaction.prepare(
            "SELECT cache_key, LENGTH(CAST(entry_json AS BLOB))
             FROM asf_search_cache
             ORDER BY updated_at_epoch DESC, cache_key DESC",
        )?;
        let mapped = stmt.query_map([], |row| {
            let key: String = row.get(0)?;
            let bytes: i64 = row.get(1)?;
            Ok((key, u64::try_from(bytes).unwrap_or(0)))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };

    let mut kept_entries: usize = 0;
    let mut kept_bytes: u64 = 0;
    let mut to_evict: Vec<String> = Vec::new();
    for (key, bytes) in rows {
        if protected_key == Some(key.as_str()) {
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
            to_evict.push(key);
        }
    }

    for key in to_evict {
        removed = removed.saturating_add(transaction.execute(
            "DELETE FROM asf_search_cache WHERE cache_key = ?1",
            params![key],
        )?);
    }
    Ok(removed)
}

fn sqlite_epoch(value: u64, label: &str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::invalid(format!("{label} exceeds SQLite range")))
}
