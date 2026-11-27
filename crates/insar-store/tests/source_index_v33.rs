use insar_core::{
    DownloadItemStatus, DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem,
    StoreCreateTaskRequest, StoreSearchBatchRequest, TypedError,
};
use insar_store::{StoreBatchPageRequest, TaskStore, STORE_SCHEMA_VERSION};
use rusqlite::Connection;
use serde_json::{json, Map};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "sp-source-index-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

/// Every schema-33 object, dropped to fake a schema-32 Store.
const DROP_V33: &str = "DROP TABLE IF EXISTS task_snapshot_heads; \
     DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; \
     DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;";

fn item(id: &str, source: Option<&str>) -> SnapshotItem {
    let mut metadata = Map::new();
    if let Some(source) = source {
        metadata.insert("source_item_id".to_owned(), json!(source));
    }
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "microsoft.planetary-computer".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn batch(id: &str, created_at: &str, collection: &str) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: id.to_owned(),
        provider_id: "microsoft.planetary-computer".to_owned(),
        created_at: created_at.to_owned(),
        aoi_name: "fixture".to_owned(),
        aoi_geojson: None,
        query:
            json!({"logical_dataset": "optical.landsat.c2.l2", "native_collection_id": collection})
                .as_object()
                .unwrap()
                .clone(),
        search_stats: None,
        candidates: (0..3).map(|i| item(&format!("item-{i}"), None)).collect(),
    }
}

fn save(store: &mut TaskStore, batch: FrozenSearchBatch) {
    save_with(store, batch, false)
}

fn save_with(store: &mut TaskStore, batch: FrozenSearchBatch, activate_workbench: bool) {
    let batch_id = batch.batch_id.clone();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id,
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 3,
                revision: 0,
            },
            activate_workbench,
        })
        .unwrap();
}

fn task_request(task_id: &str, batch_id: &str, selected: &[&str]) -> StoreCreateTaskRequest {
    StoreCreateTaskRequest {
        task_id: task_id.to_owned(),
        batch_id: batch_id.to_owned(),
        selected_item_ids: selected.iter().map(|id| (*id).to_owned()).collect(),
        checked_item_ids: vec![],
        page_index: 0,
        page_size: 3,
        output_root: PathBuf::from("fixture-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-01-01T01:00:00Z".to_owned(),
    }
}

fn page(batch_id: &str) -> StoreBatchPageRequest {
    StoreBatchPageRequest {
        batch_id: batch_id.to_owned(),
        revision: 0,
        page: 0,
        page_size: 3,
    }
}

fn fake_v32(path: &PathBuf) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(&format!("{DROP_V33} PRAGMA user_version = 32;"))
        .unwrap();
}

#[test]
fn v32_store_gains_indexes_and_backfill_is_reentrant() {
    let db = Temp::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        save(
            &mut store,
            batch("old", "2026-01-01T00:00:00Z", "landsat-c2-l2"),
        );
        save(
            &mut store,
            batch("new", "2026-01-02T00:00:00Z", "landsat-c2-l2"),
        );
        // Explicit file tasks are admitted from the active legacy selection.
        save_with(
            &mut store,
            batch("files", "2026-01-01T00:00:00Z", "other"),
            true,
        );
        store
            .create_task(&task_request("direct-task", "old", &["item-1"]))
            .unwrap();
        store
            .create_task_with_explicit_items(
                &task_request("file-task", "files", &["asset-2"]),
                0,
                &["item-2".to_owned()],
                &[item("asset-2", Some("item-2"))],
            )
            .unwrap();
    }
    fake_v32(&db.0);
    {
        let store = TaskStore::open(&db.0).unwrap();
        assert_eq!(store.health().unwrap().schema_version, 43);
        assert_eq!(STORE_SCHEMA_VERSION, 43);
        let tables = store.table_names().unwrap();
        for table in [
            "batch_task_items",
            "search_batch_headers",
            "source_views",
            "task_item_rebinds",
            "task_snapshot_heads",
        ] {
            assert!(tables.iter().any(|name| name == table), "{table}");
        }
        let old = store.load_batch_page(&page("old")).unwrap();
        assert_eq!(old.rows[1].task_id.as_deref(), Some("direct-task"));
        assert_eq!(old.rows[0].task_id, None);
        let files = store.load_batch_page(&page("files")).unwrap();
        assert_eq!(files.rows[2].task_id.as_deref(), Some("file-task"));
        assert_eq!(files.rows[2].item_status, Some(DownloadItemStatus::Queued));
        let headers = store.latest_frozen_batch_headers_per_route().unwrap();
        let landsat = headers
            .iter()
            .filter(|header| header.native_collection_id.as_deref() == Some("landsat-c2-l2"))
            .collect::<Vec<_>>();
        assert_eq!(landsat.len(), 1, "one latest header per route key");
        assert_eq!(landsat[0].batch_id, "new");
        assert_eq!(landsat[0].item_count, 3);
    }
    // Re-entrant: an interrupted or repeated upgrade over existing rows succeeds.
    {
        let connection = Connection::open(&db.0).unwrap();
        connection
            .execute_batch("PRAGMA user_version = 32;")
            .unwrap();
    }
    let store = TaskStore::open(&db.0).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    let index_rows: i64 = store
        .load_batch_page(&page("old"))
        .unwrap()
        .rows
        .iter()
        .filter(|row| row.task_id.is_some())
        .count() as i64;
    assert_eq!(index_rows, 1);
}

#[test]
fn active_task_lookup_uses_the_reverse_index_for_whole_batch() {
    let db = Temp::new();
    let mut store = TaskStore::open(&db.0).unwrap();
    save(
        &mut store,
        batch("b", "2026-01-01T00:00:00Z", "landsat-c2-l2"),
    );
    store
        .create_task(&task_request("t", "b", &["item-2"]))
        .unwrap();
    let found = store
        .active_task_for_products("b", &["item-2".to_owned()])
        .unwrap()
        .expect("whole-batch reuse");
    assert_eq!(found.snapshot.task_id, "t");
    assert!(store
        .active_task_for_products("b", &["item-0".to_owned()])
        .unwrap()
        .is_none());
}

#[test]
fn source_view_pending_success_and_error_survive_reopen_without_moving_backward() {
    let db = Temp::new();
    let error = TypedError {
        code: "CATALOGUE_TRANSPORT_FAILURE".to_owned(),
        message_zh: "网络或服务暂不可用，请稍后重试检索".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    };
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        save_with(
            &mut store,
            batch("old", "2026-01-01T00:00:00Z", "landsat-c2-l2"),
            true,
        );
        save_with(
            &mut store,
            batch("new", "2026-01-02T00:00:00Z", "landsat-c2-l2"),
            true,
        );
        let at = "2026-01-03T00:00:00Z";
        store
            .mark_source_view_pending("science.landsat", "req-1", at)
            .unwrap();
        store
            .record_source_view_success("science.landsat", "new", Some("req-1"), at)
            .unwrap();
        // A later success for an older batch never moves the pointer backward.
        store
            .record_source_view_success("science.landsat", "old", None, at)
            .unwrap();
        store
            .mark_source_view_pending("science.landsat", "req-2", at)
            .unwrap();
        store
            .record_source_view_failure("science.landsat", "req-2", &error, at)
            .unwrap();
        store
            .mark_source_view_pending("science.landsat", "req-3", at)
            .unwrap();
        assert!(store
            .record_source_view_success("science.landsat", "missing", None, at)
            .is_err());
    }
    let store = TaskStore::open(&db.0).unwrap();
    let views = store.load_source_views().unwrap();
    assert_eq!(views.len(), 1);
    let view = &views[0];
    assert_eq!(view.resource_id, "science.landsat");
    assert_eq!(view.batch_id.as_deref(), Some("new"));
    assert_eq!(view.item_count, 3);
    assert_eq!(view.searched_at.as_deref(), Some("2026-01-02T00:00:00Z"));
    assert_eq!(view.last_error.as_ref(), Some(&error));
    assert_eq!(view.pending_request_id.as_deref(), Some("req-3"));
    drop(store);
    let mut store = TaskStore::open(&db.0).unwrap();
    store
        .clear_source_view_pending("science.landsat", "req-3", "2026-01-04T00:00:00Z")
        .unwrap();
    let view = store.load_source_views().unwrap().remove(0);
    assert_eq!(view.pending_request_id, None);
    assert_eq!(view.last_error.as_ref(), Some(&error), "cancel keeps error");
}

fn latest_landsat(store: &TaskStore) -> Vec<String> {
    store
        .latest_frozen_batch_headers_per_route()
        .unwrap()
        .into_iter()
        .filter(|header| header.native_collection_id.as_deref() == Some("landsat-c2-l2"))
        .map(|header| header.batch_id)
        .collect()
}

#[test]
fn derived_and_fallback_batches_never_become_source_views() {
    let db = Temp::new();
    let mut store = TaskStore::open(&db.0).unwrap();
    save_with(
        &mut store,
        batch("search", "2026-01-01T00:00:00Z", "landsat-c2-l2"),
        true,
    );
    let mut derived = batch("derived", "2026-01-02T00:00:00Z", "landsat-c2-l2");
    derived
        .query
        .insert("derived_from_batch_id".to_owned(), json!("search"));
    save(&mut store, derived);
    save(
        &mut store,
        batch("batch-fallback-1", "2026-01-03T00:00:00Z", "landsat-c2-l2"),
    );
    assert_eq!(latest_landsat(&store), ["search"]);
    let at = "2026-01-04T00:00:00Z";
    store
        .record_source_view_success("science.landsat", "search", None, at)
        .unwrap();
    for batch_id in ["derived", "batch-fallback-1"] {
        assert!(store
            .record_source_view_success("science.landsat", batch_id, None, at)
            .is_err());
    }
    drop(store);
    let store = TaskStore::open(&db.0).unwrap();
    let view = store.load_source_views().unwrap().remove(0);
    assert_eq!(view.batch_id.as_deref(), Some("search"));
    assert_eq!(view.searched_at.as_deref(), Some("2026-01-01T00:00:00Z"));
}

#[test]
fn backfill_excludes_derived_batches_from_source_views() {
    let db = Temp::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        save(
            &mut store,
            batch("plain", "2026-01-01T00:00:00Z", "landsat-c2-l2"),
        );
        let mut derived = batch("derived", "2026-01-02T00:00:00Z", "landsat-c2-l2");
        derived
            .query
            .insert("derived_from_batch_id".to_owned(), json!("plain"));
        save(&mut store, derived);
    }
    fake_v32(&db.0);
    let store = TaskStore::open(&db.0).unwrap();
    assert_eq!(latest_landsat(&store), ["plain"]);
}

fn user_version(path: &PathBuf) -> i64 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
fn backfill_skips_undecodable_tasks_and_records_the_count() {
    let db = Temp::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        save(
            &mut store,
            batch("b", "2026-01-01T00:00:00Z", "landsat-c2-l2"),
        );
        store
            .create_task(&task_request("good", "b", &["item-0"]))
            .unwrap();
        store
            .create_task(&task_request("bad", "b", &["item-1"]))
            .unwrap();
    }
    fake_v32(&db.0);
    Connection::open(&db.0)
        .unwrap()
        .execute(
            "UPDATE task_runtime SET state_json = '{\"unexpected\":true}' WHERE task_id = 'bad'",
            [],
        )
        .unwrap();
    assert!(TaskStore::open(&db.0).is_ok());
    assert_eq!(user_version(&db.0), 43);
    let skipped: i64 = Connection::open(&db.0)
        .unwrap()
        .query_row(
            "SELECT skipped_task_count FROM source_index_backfill WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("backfill outcome is recorded");
    assert_eq!(skipped, 1);
}

#[test]
fn backfill_sqlite_error_rolls_the_migration_back() {
    let db = Temp::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        save(
            &mut store,
            batch("b", "2026-01-01T00:00:00Z", "landsat-c2-l2"),
        );
        store
            .create_task(&task_request("task", "b", &["item-0"]))
            .unwrap();
    }
    fake_v32(&db.0);
    Connection::open(&db.0)
        .unwrap()
        .execute_batch(
            "ALTER TABLE task_membership_additions RENAME COLUMN position TO moved_position;",
        )
        .unwrap();
    assert!(
        TaskStore::open(&db.0).is_err(),
        "a database error is not a skipped task"
    );
    assert_eq!(user_version(&db.0), 32, "schema-33 migration rolled back");
}
