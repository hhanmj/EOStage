//! Issue #3 R2 B5 (review): read a frozen batch's catalogue logical dataset
//! for admission gates without decoding or validating its candidates.
//! Legacy SAR batches freeze their dataset selector the same way; the
//! restricted-dataset admission gate reads it from here.
use rusqlite::OptionalExtension;

use crate::{StoreError, TaskStore};

impl TaskStore {
    /// `None` when the batch is unknown; `Some(None)` when its frozen query
    /// names no catalogue logical dataset (ASF and older batches).
    pub fn load_search_batch_logical_dataset(
        &self,
        batch_id: &str,
    ) -> Result<Option<Option<String>>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT json_extract(snapshot_json, '$.query.logical_dataset') \
                 FROM search_batches WHERE batch_id = ?1",
                [batch_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?)
    }

    /// The frozen legacy SAR query's dataset selector (e.g. `JERS-1`), or
    /// `None` for catalogue batches and unknown batch ids.
    pub fn load_search_batch_sar_dataset(
        &self,
        batch_id: &str,
    ) -> Result<Option<Option<String>>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT json_extract(snapshot_json, '$.query.dataset') \
                 FROM search_batches WHERE batch_id = ?1",
                [batch_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::TaskStore;

    #[test]
    fn reads_only_the_frozen_query_dataset() {
        let path = std::env::temp_dir().join(format!(
            "sceneport-batch-dataset-{}.sqlite3",
            std::process::id()
        ));
        let store = TaskStore::open(&path).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO search_batches (batch_id, provider_id, created_at, snapshot_json) \
                 VALUES ('hls', 'p', 't', ?1), ('asf', 'p', 't', ?2)",
                [
                    r#"{"query":{"logical_dataset":"optical.hls.l30.v2"},"candidates":[]}"#,
                    r#"{"query":{"dataset":"SENTINEL-1"},"candidates":[]}"#,
                ],
            )
            .unwrap();
        assert_eq!(
            store.load_search_batch_logical_dataset("hls").unwrap(),
            Some(Some("optical.hls.l30.v2".to_owned()))
        );
        assert_eq!(
            store.load_search_batch_logical_dataset("asf").unwrap(),
            Some(None)
        );
        assert_eq!(
            store.load_search_batch_logical_dataset("missing").unwrap(),
            None
        );
        assert_eq!(
            store.load_search_batch_sar_dataset("hls").unwrap(),
            Some(None)
        );
        assert_eq!(
            store.load_search_batch_sar_dataset("asf").unwrap(),
            Some(Some("SENTINEL-1".to_owned()))
        );
        assert_eq!(
            store.load_search_batch_sar_dataset("missing").unwrap(),
            None
        );
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
