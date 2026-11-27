use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    embed_request_manifest, parse_request_manifest, DownloadLayout, FrozenSearchBatch,
    NisarSubsetRequestManifestV1, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, NISAR_REMOTE_SUBSET_METADATA_KEY,
};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-nisar-subset-{}-{nonce}-{seq}.sqlite3",
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

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn batch() -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nisar-source-batch".to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        created_at: "2024-01-15T00:00:00Z".to_owned(),
        aoi_name: "NISAR AOI".to_owned(),
        aoi_geojson: Some(map(json!({
            "type": "Polygon",
            "coordinates": [[[73.5, 18.2], [134.8, 18.2], [134.8, 53.6], [73.5, 53.6], [73.5, 18.2]]]
        }))),
        query: map(json!({"product_type": "RSLC", "batch": "nisar-source-batch"})),
        search_stats: None,
        candidates: (0..3)
            .map(|index| SnapshotItem {
                item_id: format!("NISAR-{index:03}"),
                provider_id: "asf.nasa.earthdata".to_owned(),
                display_name: format!("NISAR candidate {index}"),
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

fn workbench() -> SearchWorkbenchState {
    SearchWorkbenchState {
        batch_id: "nisar-source-batch".to_owned(),
        checked_item_ids: vec!["NISAR-000".to_owned()],
        page_index: 0,
        page_size: 24,
        revision: 0,
    }
}

fn request_manifest() -> NisarSubsetRequestManifestV1 {
    let path = format!(
        "{}/../../docs/fixtures/asf-nisar-subset-p1/request-manifest-v1.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn nisar_contract_task_store_sqlite_round_trip_keeps_source_candidates_immutable() {
    let database = TempDatabase::new();
    let output = database
        .path()
        .parent()
        .unwrap()
        .join("nisar-subset-output");
    let source_batch = batch();
    let source_candidates = source_batch.candidates.clone();
    let request = request_manifest();
    let mut derived_batch = source_batch.clone();
    derived_batch.batch_id = "nisar-subset-contract-batch".to_owned();
    embed_request_manifest(&mut derived_batch.candidates[0].metadata, &request).unwrap();

    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: source_batch,
                workbench: workbench(),
            })
            .unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                batch: derived_batch,
                workbench: SearchWorkbenchState {
                    batch_id: "nisar-subset-contract-batch".to_owned(),
                    checked_item_ids: vec!["NISAR-000".to_owned()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .unwrap();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "nisar-subset-task".to_owned(),
                batch_id: "nisar-subset-contract-batch".to_owned(),
                selected_item_ids: vec!["NISAR-000".to_owned()],
                checked_item_ids: vec!["NISAR-000".to_owned()],
                page_index: 0,
                page_size: 24,
                output_root: output,
                layout: DownloadLayout::Direct,
                concurrency: 3,
                created_at: "2024-01-15T01:00:00Z".to_owned(),
            })
            .unwrap();
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    let task = reopened.load_task("nisar-subset-task").unwrap().unwrap();
    let restored_workbench = reopened.load_workbench().unwrap().unwrap();
    assert_eq!(restored_workbench.batch.candidates, source_candidates);
    assert!(restored_workbench
        .batch
        .candidates
        .iter()
        .all(|item| { !item.metadata.contains_key(NISAR_REMOTE_SUBSET_METADATA_KEY) }));
    let persisted = parse_request_manifest(&task.snapshot.candidates[0].metadata).unwrap();
    assert_eq!(persisted, request);
    assert_eq!(task.snapshot.initial_selected_item_ids, ["NISAR-000"]);
    assert_eq!(task.runtime.checked_item_ids, ["NISAR-000"]);
}

#[test]
fn nisar_aoi_float_identity_survives_sqlite_round_trip() {
    let database = TempDatabase::new();
    let mut source_batch = batch();
    source_batch.aoi_geojson = Some(map(json!({
        "type": "Polygon",
        "coordinates": [[
            [37.720222204639484, 10.857787104635449],
            [37.721296026021705, 10.857533983552843],
            [37.721144257142576, 10.85690570018474],
            [37.720070447767895, 10.857158813483217],
            [37.720222204639484, 10.857787104635449]
        ]]
    })));
    let expected_aoi = source_batch.aoi_geojson.clone();

    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: source_batch,
                workbench: workbench(),
            })
            .unwrap();
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened.load_workbench().unwrap().unwrap();
    assert_eq!(restored.batch.aoi_geojson, expected_aoi);
}

fn assert_task_type_projection(
    dataset: &str,
    provider: &str,
    subset: bool,
    archived: bool,
    expected: Option<&str>,
) {
    use insar_core::{
        StoreArchiveTaskRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskStateCommandKind,
    };
    use insar_store::StoreTaskHistoryPageRequest;

    let database = TempDatabase::new();
    let mut source = batch();
    source.provider_id = provider.to_owned();
    source.query = map(json!({"logical_dataset": dataset}));
    for item in &mut source.candidates {
        item.provider_id = provider.to_owned();
        item.metadata = map(json!({"logical_dataset": dataset}));
        if subset {
            item.metadata.insert(
                "delivery_kind".to_owned(),
                json!("remote_derived_subset_v2"),
            );
        }
    }
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: source,
            workbench: workbench(),
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "projection-task".to_owned(),
            batch_id: "nisar-source-batch".to_owned(),
            selected_item_ids: vec!["NISAR-000".to_owned()],
            checked_item_ids: vec!["NISAR-000".to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: database.path().parent().unwrap().join("projection-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-13T00:00:00Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "projection-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec!["NISAR-000".to_owned()],
            },
            event_id: "projection-failure".to_owned(),
            occurred_at: "2026-10-13T00:01:00Z".to_owned(),
        })
        .unwrap();
    if archived {
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "projection-task".to_owned(),
                expected_revision: 1,
                archived_at: "2026-10-13T00:02:00Z".to_owned(),
            })
            .unwrap();
    }
    drop(store);
    let reopened = TaskStore::open(database.path()).unwrap();
    let summary = if archived {
        reopened
            .load_task_history_page(&StoreTaskHistoryPageRequest {
                page_size: 10,
                after_archived_at_epoch_ms: None,
                after_task_id: None,
            })
            .unwrap()
            .items
            .remove(0)
            .task
    } else {
        reopened.core_restore(false).unwrap().tasks.remove(0)
    };
    assert_eq!(summary.dataset_id.as_deref(), Some(dataset));
    assert_eq!(summary.failed_count, 1);
    assert_eq!(
        summary.task_type.as_deref(),
        expected,
        "dataset={dataset}, archived={archived}, subset={subset}"
    );
    if archived {
        assert!(reopened.core_restore(false).unwrap().tasks.is_empty());
    }
}

#[test]
fn sband_failed_subset_keeps_authoritative_type_after_restart() {
    assert_task_type_projection(
        "sar.nisar.s.ssar.rslc.v1",
        "isro.nrsc.bhoonidhi",
        true,
        false,
        Some("nisar_subset"),
    );
}

#[test]
fn sband_archived_subset_keeps_authoritative_type_without_reactivation() {
    assert_task_type_projection(
        "sar.nisar.s.ssar.rslc.v1",
        "isro.nrsc.bhoonidhi",
        true,
        true,
        Some("nisar_subset"),
    );
}

#[test]
fn nisar_task_type_projection_preserves_full_product_lband_and_unrelated_tasks() {
    for archived in [false, true] {
        for dataset in [
            "sar.nisar.l.rslc.provisional.v1",
            "sar.nisar.s.ssar.rslc.v1",
        ] {
            assert_task_type_projection(
                dataset,
                "asf.nasa.earthdata",
                false,
                archived,
                Some("nisar_complete"),
            );
        }
        assert_task_type_projection(
            "sar.nisar.l.rslc.provisional.v1",
            "asf.nasa.earthdata",
            true,
            archived,
            Some("nisar_subset"),
        );
        assert_task_type_projection(
            "sar.sentinel1.slc.v1",
            "asf.sentinel1",
            true,
            archived,
            None,
        );
        assert_task_type_projection(
            "sar.nisar.s.unknown.v1",
            "isro.nrsc.bhoonidhi",
            true,
            archived,
            None,
        );
    }
}
