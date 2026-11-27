//! Store task event of an identity-verified NISAR workspace rebinding.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest,
};
use insar_store::{
    StoreError, StoreNisarWorkspaceRebindingEventV2Request, StoreTaskEventPageRequest, TaskStore,
    NISAR_WORKSPACE_REBINDING_EVENT_TYPE,
};
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-workspace-rebinding-{label}-{}-{nonce}.sqlite3",
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

fn seed_task(store: &mut TaskStore, output_root: &Path) {
    let item = SnapshotItem {
        item_id: "ITEM-A".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: "ITEM-A.h5".to_owned(),
        acquisition_datetime: Some("2026-10-16T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "rebinding-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-10-16T00:00:00Z".to_owned(),
                aoi_name: "Rebinding AOI".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "rebinding-batch".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
        })
        .expect("save batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "rebinding-task".to_owned(),
            batch_id: "rebinding-batch".to_owned(),
            selected_item_ids: vec!["ITEM-A".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 100,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-16T00:00:01Z".to_owned(),
        })
        .expect("create task");
}

fn request() -> StoreNisarWorkspaceRebindingEventV2Request {
    StoreNisarWorkspaceRebindingEventV2Request {
        task_id: "rebinding-task".to_owned(),
        item_id: "ITEM-A".to_owned(),
        recorded_path: r"\\?\C:\Data\EOStage-old\out\.nisar-subset-v2-recovery\ws".to_owned(),
        current_path: r"\\?\C:\Data\EOStage\out\.nisar-subset-v2-recovery\ws".to_owned(),
        identity: "windows:3405691582:281474976710700".to_owned(),
        volume_file_system: Some("NTFS".to_owned()),
        planning_checkpoint_sha256: Some("a".repeat(64)),
        science_checkpoint_sha256: Some("b".repeat(64)),
        anchor_record_sha256: Some("c".repeat(64)),
        anchor_generation: Some(3),
        occurred_at: "2026-10-17T00:00:00Z".to_owned(),
    }
}

fn revision(store: &TaskStore) -> u64 {
    store
        .load_task_record("rebinding-task")
        .unwrap()
        .unwrap()
        .runtime
        .revision
}

/// The first accepted rebinding is recorded once as a task event with the
/// local audit payload; a repeat changes nothing; the bounded event page
/// shows a localized description and never the paths.
#[test]
fn workspace_rebinding_event_is_recorded_once_and_the_page_never_projects_paths() {
    let database = TempDatabase::new("once");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    let before = revision(&store);
    assert!(store
        .record_nisar_workspace_rebinding_event_v2(&request())
        .unwrap());
    assert_eq!(revision(&store), before + 1);
    assert!(!store
        .record_nisar_workspace_rebinding_event_v2(&request())
        .unwrap());
    assert_eq!(revision(&store), before + 1, "a repeat changes nothing");
    let events = store
        .list_task_events("rebinding-task")
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == NISAR_WORKSPACE_REBINDING_EVENT_TYPE)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].item_id.as_deref(), Some("ITEM-A"));
    assert_eq!(events[0].sequence, before + 1);
    let payload = &events[0].payload;
    assert_eq!(
        payload["contract"],
        json!("nisar_workspace_rebinding_event_v2")
    );
    assert_eq!(payload["recorded_path"], json!(request().recorded_path));
    assert_eq!(payload["current_path"], json!(request().current_path));
    assert_eq!(payload["identity"], json!(request().identity));
    assert_eq!(payload["volume_file_system"], json!("NTFS"));
    assert_eq!(payload["anchor_generation"], json!(3));
    let page = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "rebinding-task".to_owned(),
            expected_revision: before + 1,
            before_sequence: None,
            page_size: 10,
        })
        .unwrap();
    let item = page
        .items
        .iter()
        .find(|item| item.sequence == before + 1)
        .expect("projected event");
    assert_eq!(
        item.message_zh,
        "已核对工作区目录身份，沿用改名前记录的路径继续"
    );
    let serialized = serde_json::to_string(&page).unwrap();
    assert!(
        !serialized.contains("EOStage-old")
            && !serialized.contains("EOStage")
            && !serialized.contains("nisar-subset-v2-recovery")
    );
}

#[test]
fn workspace_rebinding_event_rejects_foreign_items_unknown_tasks_and_unmoved_paths() {
    let database = TempDatabase::new("reject");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    let before = revision(&store);
    let mut foreign = request();
    foreign.item_id = "ITEM-B".to_owned();
    assert!(store
        .record_nisar_workspace_rebinding_event_v2(&foreign)
        .is_err());
    let mut unknown = request();
    unknown.task_id = "missing-task".to_owned();
    assert!(matches!(
        store.record_nisar_workspace_rebinding_event_v2(&unknown),
        Err(StoreError::NotFound)
    ));
    let mut unmoved = request();
    unmoved.current_path = unmoved.recorded_path.clone();
    assert!(store
        .record_nisar_workspace_rebinding_event_v2(&unmoved)
        .is_err());
    assert_eq!(revision(&store), before);
}
