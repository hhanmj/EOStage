//! B15 (2026-10-31 owner feedback): task-scoped, user-confirmed cleanup of
//! renamed-aside conflict files, through the ApplicationService boundary.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_application_service::ApplicationService;
use insar_application_service::ApplicationServiceConfig;
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("insar-b15-svc-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

/// One direct-layout task with a single `a.zip` item, plus the conflict files
/// its directory actually holds.
fn seed_service(label: &str) -> (ApplicationService, TempRoot) {
    let root = TempRoot::new(label);
    let mut store_path = root.path().to_path_buf();
    store_path.push("tasks.sqlite3");
    let mut store = TaskStore::open(&store_path).unwrap();
    let item = SnapshotItem {
        item_id: "B15-ITEM".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: "a.zip".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: object(json!({"file_name": "a.zip"})),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "b15-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-11-02T00:00:00Z".to_owned(),
                aoi_name: "B15".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "b15-batch".to_owned(),
                checked_item_ids: vec!["B15-ITEM".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "b15-task".to_owned(),
            batch_id: "b15-batch".to_owned(),
            selected_item_ids: vec!["B15-ITEM".to_owned()],
            checked_item_ids: vec!["B15-ITEM".to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: root.path().to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-11-02T00:00:01Z".to_owned(),
        })
        .unwrap();
    // A freshly created task is queued (active); pause it so the cleanup
    // surface is reachable, mirroring the UI flow.
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "b15-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: "b15-pause".to_owned(),
            occurred_at: "2026-11-02T00:00:02Z".to_owned(),
        })
        .unwrap();
    drop(store);
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store_path));
    (service, root)
}

fn write_conflict(root: &Path, name: &str, bytes: &[u8]) -> (u64, u64) {
    let path = root.join(name);
    fs::write(&path, bytes).unwrap();
    let mtime = fs::metadata(&path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    (bytes.len() as u64, mtime)
}

#[test]
fn lists_and_deletes_the_tasks_own_conflict_files() {
    let (service, root) = seed_service("list-delete");
    let (size, mtime) = write_conflict(
        root.path(),
        "a.zip.part.eostage-conflict-20261020T000000Z",
        b"12345",
    );
    // Another task's directory inside the same root is not this task's target.
    fs::create_dir_all(root.path().join("theirs")).unwrap();
    write_conflict(
        &root.path().join("theirs"),
        "a.zip.part.eostage-conflict-20261020T000000Z",
        b"other",
    );

    let listed = service.list_task_conflict_files("b15-task").unwrap();
    assert_eq!(listed.task_id, "b15-task");
    assert_eq!(listed.total_count, 1);
    assert_eq!(
        listed.files[0].path,
        "a.zip.part.eostage-conflict-20261020T000000Z"
    );
    assert_eq!(listed.files[0].size_bytes, size);
    assert_eq!(listed.files[0].modified_at_epoch_ms, mtime);
    assert!(!listed.files[0].recorded);

    let result = service
        .delete_task_conflict_files(
            "b15-task",
            &[(
                "a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
                size,
                mtime,
            )],
        )
        .unwrap();
    assert_eq!(result.outcomes.len(), 1);
    assert_eq!(result.outcomes[0].outcome, "deleted");
    assert!(!root
        .path()
        .join("a.zip.part.eostage-conflict-20261020T000000Z")
        .exists());
    assert!(root
        .path()
        .join("theirs")
        .join("a.zip.part.eostage-conflict-20261020T000000Z")
        .exists());
}

#[test]
fn unknown_task_is_not_found() {
    let (service, _root) = seed_service("unknown");
    assert!(service.list_task_conflict_files("missing-task").is_err());
    assert!(service
        .delete_task_conflict_files("missing-task", &[])
        .is_err());
}

#[test]
fn an_active_task_refuses_cleanup_until_it_pauses_or_finishes() {
    let (service, root) = seed_service("active");
    // seed_service pauses the task; a freshly created (queued) task is active
    // and must be refused on both surfaces.
    let mut store_path = root.path().to_path_buf();
    store_path.push("tasks.sqlite3");
    let mut store = TaskStore::open(&store_path).unwrap();
    let item = SnapshotItem {
        item_id: "B15-QUEUED".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: "q.zip".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: object(json!({"file_name": "q.zip"})),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: false,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "b15-batch2".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-11-02T00:00:00Z".to_owned(),
                aoi_name: "B15".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "b15-batch2".to_owned(),
                checked_item_ids: vec!["B15-QUEUED".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "b15-queued".to_owned(),
            batch_id: "b15-batch2".to_owned(),
            selected_item_ids: vec!["B15-QUEUED".to_owned()],
            checked_item_ids: vec!["B15-QUEUED".to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: root.path().to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-11-02T00:00:01Z".to_owned(),
        })
        .unwrap();
    drop(store);
    let error = service.list_task_conflict_files("b15-queued").unwrap_err();
    assert!(format!("{error:?}").contains("task is active"));
    let error = service
        .delete_task_conflict_files("b15-queued", &[])
        .unwrap_err();
    assert!(format!("{error:?}").contains("task is active"));
}

#[test]
fn a_changed_file_is_skipped_not_deleted() {
    let (service, root) = seed_service("changed");
    let (_size, _mtime) = write_conflict(
        root.path(),
        "a.zip.part.eostage-conflict-20261020T000000Z",
        b"12345",
    );
    let result = service
        .delete_task_conflict_files(
            "b15-task",
            &[(
                "a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
                999,
                1,
            )],
        )
        .unwrap();
    assert_eq!(result.outcomes[0].outcome, "skipped_changed");
    assert!(root
        .path()
        .join("a.zip.part.eostage-conflict-20261020T000000Z")
        .exists());
}

#[test]
fn paused_task_without_created_output_root_or_with_colon_item_id_returns_empty_list() {
    let (service, root) = seed_service("paused-no-dir");
    let mut store_path = root.path().to_path_buf();
    store_path.push("tasks.sqlite3");
    let mut store = TaskStore::open(&store_path).unwrap();
    let item = SnapshotItem {
        item_id: "asf.nasa.earthdata:C2853145197-ASF:NISAR_L1_PR_RSLC_001".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: "NISAR_L1_PR_RSLC_001".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: false,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "b15-nisar-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-11-02T00:00:00Z".to_owned(),
                aoi_name: "B15".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "b15-nisar-batch".to_owned(),
                checked_item_ids: vec![
                    "asf.nasa.earthdata:C2853145197-ASF:NISAR_L1_PR_RSLC_001".to_owned()
                ],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    let missing_output_root = root.path().join("not-yet-created-on-disk");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "b15-nisar-paused".to_owned(),
            batch_id: "b15-nisar-batch".to_owned(),
            selected_item_ids: vec![
                "asf.nasa.earthdata:C2853145197-ASF:NISAR_L1_PR_RSLC_001".to_owned()
            ],
            checked_item_ids: vec![
                "asf.nasa.earthdata:C2853145197-ASF:NISAR_L1_PR_RSLC_001".to_owned()
            ],
            page_index: 0,
            page_size: 24,
            output_root: missing_output_root,
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-11-02T00:00:01Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "b15-nisar-paused".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: "b15-nisar-pause".to_owned(),
            occurred_at: "2026-11-02T00:00:02Z".to_owned(),
        })
        .unwrap();
    drop(store);

    let listed = service
        .list_task_conflict_files("b15-nisar-paused")
        .unwrap();
    assert_eq!(listed.task_id, "b15-nisar-paused");
    assert_eq!(listed.total_count, 0);
    assert!(listed.files.is_empty());
}
