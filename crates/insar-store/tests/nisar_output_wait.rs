//! Store task event of a long NISAR V2 output wait (downloader enhancement
//! D1b, review of D1/D2 P2-3).
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest,
};
use insar_store::{
    StoreError, StoreNisarOutputWaitEventV2Request, StoreNisarOutputWaitReasonV2,
    StoreTaskEventPageRequest, TaskStore,
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
            "insar-output-wait-{label}-{}-{nonce}.sqlite3",
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
                batch_id: "output-wait-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-10-16T00:00:00Z".to_owned(),
                aoi_name: "Output wait AOI".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "output-wait-batch".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
        })
        .expect("save batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "output-wait-task".to_owned(),
            batch_id: "output-wait-batch".to_owned(),
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

fn request(reason: StoreNisarOutputWaitReasonV2) -> StoreNisarOutputWaitEventV2Request {
    StoreNisarOutputWaitEventV2Request {
        task_id: "output-wait-task".to_owned(),
        item_id: "ITEM-A".to_owned(),
        reason,
        holder_pid: Some(4242),
        waited_seconds: 300,
        wait_started_at: "2026-10-18T00:00:00Z".to_owned(),
        occurred_at: "2026-10-18T00:05:00Z".to_owned(),
    }
}

fn revision(store: &TaskStore) -> u64 {
    store
        .load_task_record("output-wait-task")
        .unwrap()
        .unwrap()
        .runtime
        .revision
}

/// A long wait is recorded once per wait with the reason as its event type
/// and the holder in the local payload; the event page shows a warning that
/// names the reason and never the process id.
#[test]
fn a_long_output_wait_is_recorded_once_and_the_page_names_the_reason() {
    let database = TempDatabase::new("once");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    let before = revision(&store);
    let earlier_build = request(StoreNisarOutputWaitReasonV2::EarlierBuild);
    assert!(store
        .record_nisar_output_wait_event_v2(&earlier_build)
        .unwrap());
    assert_eq!(revision(&store), before + 1);
    assert!(!store
        .record_nisar_output_wait_event_v2(&earlier_build)
        .unwrap());
    assert_eq!(revision(&store), before + 1, "a repeat changes nothing");
    let events = store
        .list_task_events("output-wait-task")
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "nisar_output_wait_earlier_build")
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].item_id.as_deref(), Some("ITEM-A"));
    let payload = &events[0].payload;
    assert_eq!(payload["contract"], json!("nisar_output_wait_event_v2"));
    assert_eq!(payload["reason"], json!("earlier_build"));
    assert_eq!(payload["holder_pid"], json!(4242));
    assert_eq!(payload["waited_seconds"], json!(300));
    let page = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "output-wait-task".to_owned(),
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
    assert_eq!(item.level, "warning");
    assert_eq!(
        item.message_zh,
        "写出已等待超过 5 分钟：旧版本程序正在向同一输出目录写出"
    );
    assert!(!serde_json::to_string(&page).unwrap().contains("4242"));
}

/// Each reason has its own description, so the task log tells them apart.
#[test]
fn every_output_wait_reason_has_its_own_description() {
    let database = TempDatabase::new("reasons");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    let mut messages = Vec::new();
    for (index, reason) in [
        StoreNisarOutputWaitReasonV2::SameArtifactInProcess,
        StoreNisarOutputWaitReasonV2::SameArtifactOtherProcess,
        StoreNisarOutputWaitReasonV2::EarlierBuild,
        StoreNisarOutputWaitReasonV2::OutputSlot,
    ]
    .into_iter()
    .enumerate()
    {
        let mut wait = request(reason);
        wait.wait_started_at = format!("2026-10-18T00:0{index}:00Z");
        assert!(store.record_nisar_output_wait_event_v2(&wait).unwrap());
    }
    let page = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "output-wait-task".to_owned(),
            expected_revision: revision(&store),
            before_sequence: None,
            page_size: 20,
        })
        .unwrap();
    for item in &page.items {
        if item.message_zh.starts_with("写出已等待超过 5 分钟") {
            messages.push(item.message_zh.clone());
        }
    }
    messages.sort();
    messages.dedup();
    assert_eq!(messages.len(), 4, "{messages:?}");
}

#[test]
fn a_long_output_wait_rejects_foreign_items_and_unknown_tasks() {
    let database = TempDatabase::new("reject");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    let before = revision(&store);
    let mut foreign = request(StoreNisarOutputWaitReasonV2::OutputSlot);
    foreign.item_id = "ITEM-B".to_owned();
    assert!(store.record_nisar_output_wait_event_v2(&foreign).is_err());
    let mut unknown = request(StoreNisarOutputWaitReasonV2::OutputSlot);
    unknown.task_id = "missing-task".to_owned();
    assert!(matches!(
        store.record_nisar_output_wait_event_v2(&unknown),
        Err(StoreError::NotFound)
    ));
    let mut empty = request(StoreNisarOutputWaitReasonV2::OutputSlot);
    empty.wait_started_at = String::new();
    assert!(store.record_nisar_output_wait_event_v2(&empty).is_err());
    assert_eq!(revision(&store), before);
}
