use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest,
};
use insar_store::TaskStore;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

struct TempDatabase(PathBuf);

static NEXT_TEMP_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

impl TempDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_TEMP_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-store-size-{}-{nonce}-{serial}.sqlite3",
            std::process::id(),
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn seed(store: &mut TaskStore, candidate_count: usize) -> FrozenSearchBatch {
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "large-search".to_string(),
        provider_id: "asf.catalog".to_string(),
        created_at: "2024-01-15T00:00:00Z".to_string(),
        aoi_name: "真实 AOI".to_string(),
        aoi_geojson: Some(map(json!({
            "type": "Polygon",
            "coordinates": [[[100.0, 20.0], [101.0, 20.0], [101.0, 21.0], [100.0, 21.0], [100.0, 20.0]]]
        }))),
        query: map(json!({"dataset": "ERS"})),
        search_stats: None,
        candidates: (0..candidate_count)
            .map(|index| SnapshotItem {
                item_id: format!("ASF::ERS::ERS-{index:05}"),
                provider_id: "asf.catalog".to_string(),
                display_name: format!("ERS-{index:05}.DAT"),
                acquisition_datetime: None,
                footprint_geojson: None,
                metadata: map(json!({"dataset": "ERS", "processing_level": "L1"})),
                payload: map(json!({
                    "file_name": format!("ERS-{index:05}.DAT"),
                    "download_url": format!("https://datapool.asf.alaska.edu/ERS/ERS-{index:05}.DAT")
                })),
                asset_access: None,
                execution_policy: None,
            })
            .collect(),
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch.clone(),
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    batch
}

fn create_request(batch: &FrozenSearchBatch, output_root: &Path) -> StoreCreateTaskRequest {
    let selected = batch
        .candidates
        .iter()
        .take(10)
        .map(|item| item.item_id.clone())
        .collect::<Vec<_>>();
    StoreCreateTaskRequest {
        task_id: "task-size".to_string(),
        batch_id: batch.batch_id.clone(),
        selected_item_ids: selected.clone(),
        checked_item_ids: selected,
        page_index: 0,
        page_size: 24,
        output_root: output_root.to_path_buf(),
        layout: DownloadLayout::SarDatasetProcessing,
        concurrency: 2,
        created_at: "2024-01-15T01:00:00Z".to_string(),
    }
}

#[test]
fn task_json_size_depends_on_selection_not_all_candidates() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    let batch = seed(&mut store, 10_000);
    store
        .create_task(&create_request(&batch, database.path().parent().unwrap()))
        .unwrap();

    let connection = Connection::open(database.path()).unwrap();
    let task_json: String = connection
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = 'task-size'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        task_json.len() < 200_000,
        "task snapshot unexpectedly duplicated the batch: {}",
        task_json.len()
    );
    assert_eq!(
        store
            .load_task("task-size")
            .unwrap()
            .unwrap()
            .snapshot
            .candidates
            .len(),
        10_000
    );
}

#[test]
fn legacy_embedded_task_snapshot_still_loads_against_its_batch() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    let batch = seed(&mut store, 3);
    let created = store
        .create_task(&create_request(&batch, database.path().parent().unwrap()))
        .unwrap();
    let legacy_json = serde_json::to_string(&created.snapshot).unwrap();
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute("DROP TRIGGER task_snapshots_are_frozen", [])
        .unwrap();
    connection
        .execute(
            "UPDATE task_snapshots SET snapshot_json = ?1 WHERE task_id = 'task-size'",
            [legacy_json],
        )
        .unwrap();

    let restored = store.load_task("task-size").unwrap().unwrap();
    assert_eq!(restored.snapshot, created.snapshot);
    assert_eq!(restored.runtime, created.runtime);
}
