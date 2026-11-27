//! Downloader D3 (local-first dedup): task page projection of local reuse
//! decisions, the reacquire_items command and historical successors created
//! to download reused items again.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    CreateTaskFromHistoricalSourceBatchRequest, DownloadLayout, FrozenSearchBatch,
    LocalReuseDisposition, LocalReuseGrade, SearchWorkbenchState, SnapshotItem,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreTaskPageRequest, StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_store::{StoreLocalReuseDisposition, StoreTaskItemLocalReuse, TaskStore};
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-local-reuse-page-{name}-{}-{nonce}.sqlite3",
            std::process::id()
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

const ITEMS: [&str; 2] = ["S1-A", "S1-B"];

fn seed(store: &mut TaskStore, output_root: &Path) {
    let candidates = ITEMS
        .iter()
        .map(|item_id| SnapshotItem {
            item_id: (*item_id).to_owned(),
            provider_id: "asf.sentinel1".to_owned(),
            display_name: format!("{item_id}.zip"),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        })
        .collect();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-reuse".to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: "2026-10-20T00:00:00Z".to_owned(),
                aoi_name: "测试区域".to_owned(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[103.0, 26.8], [104.0, 26.8], [104.0, 27.8], [103.0, 27.8], [103.0, 26.8]]]
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
                query: json!({"product_type": "SLC"}).as_object().unwrap().clone(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-reuse".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-reuse".to_owned(),
            batch_id: "batch-reuse".to_owned(),
            selected_item_ids: ITEMS.iter().map(|item| (*item).to_owned()).collect(),
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-20T00:00:01Z".to_owned(),
        })
        .unwrap();
}

fn update(
    store: &mut TaskStore,
    kind: TaskStateCommandKind,
    item_ids: &[&str],
    event: &str,
) -> Result<u64, insar_store::StoreError> {
    let revision = store
        .load_task("task-reuse")
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-reuse".to_owned(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind,
                item_ids: item_ids.iter().map(|item| (*item).to_owned()).collect(),
            },
            event_id: event.to_owned(),
            occurred_at: "2026-10-20T00:01:00Z".to_owned(),
        })
        .map(|task| task.runtime.revision)
}

fn page(store: &TaskStore) -> insar_core::StoreTaskPage {
    let revision = store
        .load_task("task-reuse")
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "task-reuse".to_owned(),
            expected_revision: revision,
            page_index: 0,
            page_size: 24,
        })
        .unwrap()
}

fn reused(item_id: &str) -> StoreTaskItemLocalReuse {
    StoreTaskItemLocalReuse {
        task_id: "task-reuse".to_owned(),
        item_id: item_id.to_owned(),
        disposition: StoreLocalReuseDisposition::Reused {
            grade: "size_and_format".to_owned(),
        },
        size_bytes: Some(42),
        decided_at: "2026-10-20T00:00:30Z".to_owned(),
    }
}

#[test]
fn the_task_page_projects_each_item_local_reuse_decision() {
    let database = TempDatabase::new("projection");
    let output = database.path().parent().unwrap().join("local-reuse-page");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, &output);
    store.record_task_item_local_reuse(&reused("S1-A")).unwrap();
    store
        .record_task_item_local_reuse(&StoreTaskItemLocalReuse {
            item_id: "S1-B".to_owned(),
            disposition: StoreLocalReuseDisposition::RenamedConflict {
                reason: "size_mismatch".to_owned(),
                conflict_file_name: "S1-B.zip.eostage-conflict-20261020T000030Z".to_owned(),
            },
            size_bytes: None,
            ..reused("S1-B")
        })
        .unwrap();
    let page = page(&store);
    let a = page.items[0]
        .local_reuse
        .clone()
        .expect("S1-A is annotated");
    assert_eq!(a.disposition, LocalReuseDisposition::Reused);
    assert_eq!(a.grade, Some(LocalReuseGrade::SizeAndFormat));
    let b = page.items[1]
        .local_reuse
        .clone()
        .expect("S1-B is annotated");
    assert_eq!(b.disposition, LocalReuseDisposition::RenamedConflict);
    assert_eq!(b.conflict_reason.as_deref(), Some("size_mismatch"));
    assert_eq!(
        b.conflict_file_name.as_deref(),
        Some("S1-B.zip.eostage-conflict-20261020T000030Z")
    );
    assert!(page.items.iter().all(|item| item.local_reuse.is_some()));
    let serialized = serde_json::to_value(&page).unwrap();
    assert_eq!(
        serialized["items"][0]["local_reuse"]["grade"],
        json!("size_and_format")
    );
}

#[test]
fn reacquire_moves_only_skipped_items_back_to_the_queue_and_records_the_request() {
    let database = TempDatabase::new("reacquire");
    let output = database
        .path()
        .parent()
        .unwrap()
        .join("local-reuse-reacquire");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, &output);
    update(
        &mut store,
        TaskStateCommandKind::MarkSkipped,
        &["S1-A"],
        "skip-a",
    )
    .unwrap();
    store.record_task_item_local_reuse(&reused("S1-A")).unwrap();
    // A queued (not skipped) item cannot be reacquired.
    assert!(update(
        &mut store,
        TaskStateCommandKind::ReacquireItems,
        &["S1-B"],
        "reacquire-b"
    )
    .is_err());
    update(
        &mut store,
        TaskStateCommandKind::ReacquireItems,
        &["S1-A"],
        "reacquire-a",
    )
    .unwrap();
    let task = store.load_task("task-reuse").unwrap().unwrap();
    assert!(task.runtime.queued_item_ids.contains(&"S1-A".to_owned()));
    assert!(!task.runtime.skipped_item_ids.contains(&"S1-A".to_owned()));
    let decision = store
        .task_item_local_reuse("task-reuse", "S1-A")
        .unwrap()
        .unwrap();
    assert_eq!(
        decision.disposition,
        StoreLocalReuseDisposition::ReacquireRequested
    );
    assert_eq!(decision.decided_at, "2026-10-20T00:01:00Z");
    assert_eq!(
        page(&store).items[0]
            .local_reuse
            .as_ref()
            .map(|value| value.disposition.clone()),
        Some(LocalReuseDisposition::ReacquireRequested)
    );
}

#[test]
fn a_historical_successor_can_download_reused_items_again() {
    let database = TempDatabase::new("successor");
    let output = database
        .path()
        .parent()
        .unwrap()
        .join("local-reuse-successor");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, &output);
    update(
        &mut store,
        TaskStateCommandKind::MarkSkipped,
        &["S1-A"],
        "skip-a",
    )
    .unwrap();
    let revision = update(
        &mut store,
        TaskStateCommandKind::MarkSkipped,
        &["S1-B"],
        "skip-b",
    )
    .unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-reuse".to_owned(),
            expected_revision: revision,
            archived_at: "2026-10-20T00:02:00Z".to_owned(),
        })
        .unwrap();
    let request = |reacquire: Vec<String>| CreateTaskFromHistoricalSourceBatchRequest {
        source_task_id: "task-reuse".to_owned(),
        expected_source_task_revision: revision,
        new_task_id: "task-reuse-again".to_owned(),
        item_ids: vec!["S1-A".to_owned()],
        output_root: output.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-10-20T00:03:00Z".to_owned(),
        reacquire_item_ids: reacquire,
    };
    assert!(
        store
            .create_task_from_historical_source_batch(&request(vec!["S1-B".to_owned()]))
            .is_err(),
        "only items of the successor can be downloaded again"
    );
    assert!(
        store.load_task("task-reuse-again").unwrap().is_none(),
        "nothing was created"
    );
    store
        .create_task_from_historical_source_batch(&request(vec!["S1-A".to_owned()]))
        .unwrap();
    let decision = store
        .task_item_local_reuse("task-reuse-again", "S1-A")
        .unwrap()
        .expect("the successor item is to be downloaded again");
    assert_eq!(
        decision.disposition,
        StoreLocalReuseDisposition::ReacquireRequested
    );
}
