use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest,
};
use insar_store::{StoreTaskFailureSummary, TaskStore};
use rusqlite::Connection;
use serde_json::Map;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-task-failure-summary-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn seed_task(store: &mut TaskStore, task_id: &str, item_count: usize) {
    let batch_id = format!("{task_id}-batch");
    let item_ids = (0..item_count)
        .map(|index| format!("{task_id}-item-{index}"))
        .collect::<Vec<_>>();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.clone(),
                provider_id: "fixture.provider".to_owned(),
                created_at: "2026-10-25T00:00:00Z".to_owned(),
                aoi_name: "fixture AOI".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: item_ids
                    .iter()
                    .map(|item_id| SnapshotItem {
                        item_id: item_id.clone(),
                        provider_id: "fixture.provider".to_owned(),
                        display_name: item_id.clone(),
                        acquisition_datetime: None,
                        footprint_geojson: None,
                        metadata: Map::new(),
                        payload: Map::new(),
                        asset_access: None,
                        execution_policy: None,
                    })
                    .collect(),
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 25,
                revision: 0,
            },
            activate_workbench: false,
        })
        .expect("save batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id,
            selected_item_ids: item_ids.clone(),
            checked_item_ids: item_ids,
            page_index: 0,
            page_size: 25,
            output_root: PathBuf::from("fixture-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-25T00:00:01Z".to_owned(),
        })
        .expect("create task");
}

/// A task-level failure without a failed item (the Store's lifecycle never
/// writes one, but an older or hand-repaired row can): reported as failed with
/// no codes and no message rather than as healthy or with an invented reason.
#[test]
fn failed_task_without_failed_items_reports_an_empty_honest_summary() {
    let database = TempDatabase::new("failed-empty");
    let mut store = TaskStore::open(&database.0).expect("open store");
    seed_task(&mut store, "failed-empty", 3);
    seed_task(&mut store, "queued", 2);
    Connection::open(&database.0)
        .expect("raw connection")
        .execute(
            "UPDATE task_runtime SET status = 'failed' WHERE task_id = 'failed-empty'",
            [],
        )
        .expect("hand-written failed status");
    let ids = ["failed-empty".to_owned(), "queued".to_owned()];
    let summaries = store
        .load_task_failure_summaries(&ids, &[])
        .expect("summaries");
    assert_eq!(
        summaries.get("failed-empty"),
        Some(&StoreTaskFailureSummary {
            codes: vec![],
            message_zh: None,
            failed_items: 0,
            total_items: 3,
        })
    );
    assert!(!summaries.contains_key("queued"));
}

/// One unreadable runtime row makes only that task's failure unknown; the
/// summaries of the other tasks are still returned.
#[test]
fn corrupt_runtime_row_degrades_only_its_own_summary() {
    let database = TempDatabase::new("corrupt-runtime");
    let mut store = TaskStore::open(&database.0).expect("open store");
    seed_task(&mut store, "corrupt", 2);
    seed_task(&mut store, "failed", 2);
    let connection = Connection::open(&database.0).expect("raw connection");
    connection
        .execute(
            "UPDATE task_runtime SET status = 'failed', state_json = '{not json' \
             WHERE task_id = 'corrupt'",
            [],
        )
        .expect("hand-written corrupt runtime row");
    connection
        .execute(
            "UPDATE task_runtime SET status = 'failed', \
             state_json = json_set(state_json, '$.failed_item_ids', json_array('failed-item-1'), \
                                   '$.queued_item_ids', json_array('failed-item-0')) \
             WHERE task_id = 'failed'",
            [],
        )
        .expect("hand-written failed item");
    let ids = ["corrupt".to_owned(), "failed".to_owned()];
    let summaries = store
        .load_task_failure_summaries(&ids, &ids)
        .expect("one corrupt row must not fail the whole read");
    assert!(!summaries.contains_key("corrupt"));
    assert_eq!(
        summaries.get("failed"),
        Some(&StoreTaskFailureSummary {
            codes: vec!["UNKNOWN_FAILURE_CODE".to_owned()],
            message_zh: None,
            failed_items: 1,
            total_items: 2,
        })
    );
}
