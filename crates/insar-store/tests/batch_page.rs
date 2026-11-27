use insar_core::{
    DownloadItemStatus, DownloadLayout, FrozenSearchBatch, RowTaskAction, SearchWorkbenchState,
    SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest, TaskLifecycleStatus,
};
use insar_store::{StoreBatchPageRequest, TaskStore};
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
            "sp-batch-page-{}-{}.sqlite3",
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
fn batch(id: &str) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: id.to_owned(),
        provider_id: "fixture.provider".to_owned(),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        aoi_name: "fixture".to_owned(),
        aoi_geojson: None,
        query: Map::new(),
        search_stats: None,
        candidates: (0..2)
            .map(|i| SnapshotItem {
                item_id: format!("item-{i}"),
                provider_id: "fixture.provider".to_owned(),
                display_name: format!("scene {i}"),
                acquisition_datetime: None,
                footprint_geojson: None,
                metadata: Map::new(),
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            })
            .collect(),
    }
}
fn product(id: &str, source: Option<&str>) -> SnapshotItem {
    let mut metadata = Map::new();
    if let Some(source) = source {
        metadata.insert("source_item_id".to_owned(), json!(source));
    }
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "fixture.provider".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}
fn page(batch_id: &str) -> StoreBatchPageRequest {
    StoreBatchPageRequest {
        batch_id: batch_id.to_owned(),
        revision: 0,
        page: 0,
        page_size: 2,
    }
}
#[test]
fn batch_page_rows_follow_store_task_after_reopen_and_source_switch() {
    let db = Temp::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        for id in ["a", "b"] {
            store
                .save_search_batch(&StoreSearchBatchRequest {
                    batch: batch(id),
                    workbench: SearchWorkbenchState {
                        batch_id: id.to_owned(),
                        checked_item_ids: vec![],
                        page_index: 0,
                        page_size: 2,
                        revision: 0,
                    },
                    activate_workbench: true,
                })
                .unwrap();
        }
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "task-a".to_owned(),
                batch_id: "a".to_owned(),
                selected_item_ids: vec!["item-1".to_owned()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 2,
                output_root: PathBuf::from("fixture-output"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-01-01T01:00:00Z".to_owned(),
            })
            .unwrap();
    }
    let reopened = TaskStore::open(&db.0).unwrap();
    let a = reopened.load_batch_page(&page("a")).unwrap();
    assert_eq!(a.total_count, 2);
    assert_eq!(a.rows.len(), 2);
    assert_eq!(a.rows[0].task_id, None);
    assert_eq!(a.rows[0].actions, vec![RowTaskAction::Download]);
    assert_eq!(a.rows[1].task_id.as_deref(), Some("task-a"));
    assert_eq!(a.rows[1].task_status, Some(TaskLifecycleStatus::Queued));
    assert_eq!(a.rows[1].item_status, Some(DownloadItemStatus::Queued));
    assert!(a.rows[1].actions.contains(&RowTaskAction::OpenTask));
    assert_eq!(
        reopened.load_batch_page(&page("b")).unwrap().rows[1].task_id,
        None
    );
}

#[test]
fn explicit_files_map_to_their_frozen_source_product_after_restart() {
    let db = Temp::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch: batch("files"),
                workbench: SearchWorkbenchState {
                    batch_id: "files".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 2,
                    revision: 0,
                },
                activate_workbench: true,
            })
            .unwrap();
        store
            .create_task_with_explicit_items(
                &StoreCreateTaskRequest {
                    task_id: "file-task".to_owned(),
                    batch_id: "files".to_owned(),
                    selected_item_ids: vec!["asset-1".to_owned()],
                    checked_item_ids: vec!["asset-1".to_owned()],
                    page_index: 0,
                    page_size: 2,
                    output_root: PathBuf::from("fixture-output"),
                    layout: DownloadLayout::Direct,
                    concurrency: 1,
                    created_at: "2026-01-01T01:00:00Z".to_owned(),
                },
                0,
                &["item-1".to_owned()],
                &[product("asset-1", Some("item-1"))],
            )
            .unwrap();
    }
    let reopened = TaskStore::open(&db.0).unwrap();
    let page = reopened.load_batch_page(&page("files")).unwrap();
    assert_eq!(page.rows[1].task_id.as_deref(), Some("file-task"));
    assert_eq!(page.rows[1].item_status, Some(DownloadItemStatus::Queued));
    assert_eq!(page.rows[0].task_id, None);
}
