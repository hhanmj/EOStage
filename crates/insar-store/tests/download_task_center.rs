use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use insar_core::{
    DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState,
    SnapshotItem, StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreDeleteHistoryRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreTaskSourceBatchPageRequest,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_store::{is_replayable_optical_delivery_kind, TaskStore};
use serde_json::{json, Map};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    root: PathBuf,
    db: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sp-dtc-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let db = root.join("store.sqlite3");
        Self { root, db }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn sar_item(id: &str) -> SnapshotItem {
    let mut payload = Map::new();
    payload.insert(
        "download_url".to_owned(),
        json!(format!("https://datapool.asf.alaska.edu/SLC/SA/{id}.zip")),
    );
    SnapshotItem {
        item_id: id.to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: id.to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload,
        asset_access: None,
        execution_policy: None,
    }
}

fn seed_two_item_task(store: &mut TaskStore, temp: &TempDir, task_id: &str, created_at: &str) {
    let batch_id = format!("batch-{task_id}");
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.clone(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: created_at.to_owned(),
                aoi_name: "Hefei".to_owned(),
                aoi_geojson: None,
                query: json!({"product_type": "SLC"}).as_object().unwrap().clone(),
                search_stats: None,
                candidates: vec![sar_item("S1-A"), sar_item("S1-B")],
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.clone(),
                checked_item_ids: vec!["S1-A".to_owned(), "S1-B".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id,
            selected_item_ids: vec!["S1-A".to_owned(), "S1-B".to_owned()],
            checked_item_ids: vec!["S1-A".to_owned(), "S1-B".to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: temp.root.join("out"),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 2,
            created_at: created_at.to_owned(),
        })
        .unwrap();
}

#[test]
fn task_pins_persist_in_order_and_clean_up_on_delete_history() {
    let temp = TempDir::new();
    let mut store = TaskStore::open(&temp.db).unwrap();
    seed_two_item_task(&mut store, &temp, "task-1", "2026-01-01T00:00:00Z");
    seed_two_item_task(&mut store, &temp, "task-2", "2026-01-02T00:00:00Z");

    let pins = store
        .set_task_pinned("task-1", true, "2026-01-03T00:00:00Z")
        .unwrap();
    assert_eq!(pins, vec!["task-1"]);
    let pins = store
        .set_task_pinned("task-2", true, "2026-01-04T00:00:00Z")
        .unwrap();
    assert_eq!(pins, vec!["task-2", "task-1"]);
    assert_eq!(
        store.list_pinned_task_ids().unwrap(),
        vec!["task-2", "task-1"]
    );

    // Unpinning task-2 leaves task-1.
    let pins = store
        .set_task_pinned("task-2", false, "2026-01-05T00:00:00Z")
        .unwrap();
    assert_eq!(pins, vec!["task-1"]);

    // Cancelling and deleting history of task-1 cleans up its pin automatically.
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-1".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelItems,
                item_ids: vec!["S1-A".to_owned(), "S1-B".to_owned()],
            },
            event_id: "cancel-1".to_owned(),
            occurred_at: "2026-01-05T01:00:00Z".to_owned(),
        })
        .unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-1".to_owned(),
            expected_revision: 1,
            archived_at: "2026-01-05T01:01:00Z".to_owned(),
        })
        .unwrap();
    store
        .delete_history(&StoreDeleteHistoryRequest {
            task_id: "task-1".to_owned(),
            deleted_at: "2026-01-05T01:02:00Z".to_owned(),
        })
        .unwrap();
    assert!(store.list_pinned_task_ids().unwrap().is_empty());
}

#[test]
fn resume_paused_task_items_selectively_resumes_single_item_when_whole_task_paused() {
    let temp = TempDir::new();
    let mut store = TaskStore::open(&temp.db).unwrap();
    seed_two_item_task(&mut store, &temp, "task-pause", "2026-01-01T00:00:00Z");

    // Pause whole task at revision 0 -> revision 1 (task_paused = true, S1-A/S1-B paused/queued).
    let paused = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-pause".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "pause-all-1".to_owned(),
            occurred_at: "2026-01-01T00:01:00Z".to_owned(),
        })
        .unwrap();
    assert!(paused.runtime.task_paused);
    assert_eq!(paused.runtime.status, TaskLifecycleStatus::Paused);

    // Resuming only S1-A keeps S1-B paused and transitions task to Running/Queued.
    let resumed = store
        .resume_paused_task_items(&StoreUpdateTaskRequest {
            task_id: "task-pause".to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeItems,
                item_ids: vec!["S1-A".to_owned()],
            },
            event_id: "resume-a-1".to_owned(),
            occurred_at: "2026-01-01T00:02:00Z".to_owned(),
        })
        .unwrap();
    assert!(!resumed.runtime.task_paused);
    assert_eq!(resumed.runtime.queued_item_ids, vec!["S1-A"]);
    assert_eq!(resumed.runtime.paused_item_ids, vec!["S1-B"]);
    assert_eq!(resumed.runtime.status, TaskLifecycleStatus::Queued);
}

#[test]
fn retry_archived_task_items_unarchives_failed_item_and_preserves_completed_sibling() {
    let temp = TempDir::new();
    let mut store = TaskStore::open(&temp.db).unwrap();
    seed_two_item_task(&mut store, &temp, "task-arch", "2026-01-01T00:00:00Z");

    let dest_a = temp.root.join("out").join("S1-A.zip");
    fs::create_dir_all(dest_a.parent().unwrap()).unwrap();
    fs::write(&dest_a, b"zip-payload").unwrap();

    // Mark S1-A completed via save_download_result, S1-B failed, then archive.
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "task-arch".to_owned(),
                item_id: "S1-A".to_owned(),
                outcome: DownloadOutcome::Completed,
                destination: dest_a.clone(),
                bytes_written: 11,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-01-01T00:01:00Z".to_owned(),
            },
            0,
        )
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-arch".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["S1-A".to_owned()],
            },
            event_id: "complete-a".to_owned(),
            occurred_at: "2026-01-01T00:01:30Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-arch".to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec!["S1-B".to_owned()],
            },
            event_id: "fail-b".to_owned(),
            occurred_at: "2026-01-01T00:02:00Z".to_owned(),
        })
        .unwrap();
    let rev = store
        .load_task("task-arch")
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-arch".to_owned(),
            expected_revision: rev,
            archived_at: "2026-01-01T00:03:00Z".to_owned(),
        })
        .unwrap();

    let destinations = store.load_completed_download_destinations().unwrap();
    assert_eq!(destinations.len(), 1);
    assert_eq!(destinations[0].task_id, "task-arch");
    assert_eq!(destinations[0].item_id, "S1-A");
    assert_eq!(destinations[0].destination, dest_a);

    // `update_task` remains rejected on archived tasks (history_tombstones contract).
    assert!(store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-arch".to_owned(),
            expected_revision: rev,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryItems,
                item_ids: vec!["S1-B".to_owned()],
            },
            event_id: "retry-b-via-update".to_owned(),
            occurred_at: "2026-01-01T00:03:30Z".to_owned(),
        })
        .is_err());

    // Retry only failed item S1-B via retry_archived_task_items -> unarchived + queued.
    let retried = store
        .retry_archived_task_items(&StoreUpdateTaskRequest {
            task_id: "task-arch".to_owned(),
            expected_revision: rev,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryItems,
                item_ids: vec!["S1-B".to_owned()],
            },
            event_id: "retry-b".to_owned(),
            occurred_at: "2026-01-01T00:04:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(retried.runtime.status, TaskLifecycleStatus::Queued);
    assert_eq!(retried.runtime.completed_item_ids, vec!["S1-A"]);
    assert_eq!(retried.runtime.queued_item_ids, vec!["S1-B"]);
    assert!(retried.runtime.failed_item_ids.is_empty());
}

#[test]
fn replayable_optical_delivery_kind_covers_expected_variants() {
    for kind in [
        "landsat_c2_l2_asset_batch",
        "landsat_c2_l2_bundle_batch",
        "landsat_c2_l2_full_bundle",
        "landsat_c2_l2_matched_full_bundle",
        "landsat_c2_l2_complete_asset_set",
        "landsat_c2_l2_asset_selection",
        "sentinel2_l2a_analysis_band_batch",
        "sentinel2_l2a_analysis_band_set",
        "sentinel2_l1c_band_batch",
        "sentinel2_cdse_safe_archive_batch",
        "sentinel2_cdse_safe_archive",
        "hls_multiband_batch",
        "planetary_computer_reviewed_cog_assets",
    ] {
        assert!(is_replayable_optical_delivery_kind(kind), "missing {kind}");
    }
    assert!(!is_replayable_optical_delivery_kind("optical_aoi_clip_v1"));
    assert!(!is_replayable_optical_delivery_kind(
        "remote_derived_subset_v2"
    ));
}

#[test]
fn load_task_source_batch_page_accepts_earth_search_sentinel2_band_batch() {
    let temp = TempDir::new();
    let mut store = TaskStore::open(&temp.db).unwrap();
    let scene_item = SnapshotItem {
        item_id: "S2A_ES_SCENE_1".to_owned(),
        provider_id: "element84.earth-search".to_owned(),
        display_name: "S2A_ES_SCENE_1".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: json!({"download_url": "https://earth-search.aws.element84.com/v1/items/s2a"})
            .as_object()
            .unwrap()
            .clone(),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "source-es-s2".to_owned(),
                provider_id: "element84.earth-search".to_owned(),
                created_at: "2026-11-23T00:00:00Z".to_owned(),
                aoi_name: "Hefei".to_owned(),
                aoi_geojson: None,
                query: json!({
                    "provider_id": "element84.earth-search",
                    "native_collection_id": "sentinel-2-l2a",
                    "logical_dataset": "optical.sentinel-2.l2a"
                })
                .as_object()
                .unwrap()
                .clone(),
                search_stats: None,
                candidates: vec![scene_item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "source-es-s2".to_owned(),
                checked_item_ids: vec!["S2A_ES_SCENE_1".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();

    let band_item = SnapshotItem {
        item_id: "S2A_ES_SCENE_1::B04".to_owned(),
        provider_id: "element84.earth-search".to_owned(),
        display_name: "B04.tif".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: json!({"source_item_id": "S2A_ES_SCENE_1"})
            .as_object()
            .unwrap()
            .clone(),
        payload:
            json!({"download_url": "https://sentinel-cogs.s3.us-west-2.amazonaws.com/b04.tif"})
                .as_object()
                .unwrap()
                .clone(),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: false,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "sentinel2-bands-batch-task-es-s2".to_owned(),
                provider_id: "element84.earth-search".to_owned(),
                created_at: "2026-11-23T00:01:00Z".to_owned(),
                aoi_name: "Hefei".to_owned(),
                aoi_geojson: None,
                query: json!({
                    "provider_id": "element84.earth-search",
                    "native_collection_id": "sentinel-2-l2a",
                    "logical_dataset": "optical.sentinel-2.l2a",
                    "delivery_kind": "sentinel2_l2a_analysis_band_batch",
                    "derived_from_batch_id": "source-es-s2",
                    "delivery_requested_count": 1,
                    "delivery_rejected_count": 0
                })
                .as_object()
                .unwrap()
                .clone(),
                search_stats: None,
                candidates: vec![band_item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "sentinel2-bands-batch-task-es-s2".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();

    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-es-s2".to_owned(),
            batch_id: "sentinel2-bands-batch-task-es-s2".to_owned(),
            selected_item_ids: vec!["S2A_ES_SCENE_1::B04".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: temp.root.join("out"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-11-23T00:01:00Z".to_owned(),
        })
        .unwrap();

    let page = store
        .load_task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "task-es-s2".to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(page.source_batch_id, "source-es-s2");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].item.item_id, "S2A_ES_SCENE_1");
}
