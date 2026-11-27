use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout,
    DownloadOutcome, FrozenAssetAccess, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreTaskPageRequest, StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_store::{
    StoreTaskActiveProgressSummary, StoreTaskEventPageRequest, StoreTaskFailureCode,
    StoreTaskHistoryPageRequest, StoreTaskOutputDirectoryRequest, StoreTaskTransferTelemetryUpdate,
    TaskStore,
};
use rusqlite::Connection;
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-task-telemetry-{label}-{}-{nonce}.sqlite3",
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
    seed_task_with_size(store, output_root, None);
}

fn seed_task_with_size(store: &mut TaskStore, output_root: &Path, size: Option<serde_json::Value>) {
    let item = SnapshotItem {
        item_id: "ITEM-A".to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: "ITEM-A.zip".to_owned(),
        acquisition_datetime: Some("2026-08-19T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: size
            .map(|value| Map::from_iter([("file_size_remote".to_owned(), value)]))
            .unwrap_or_default(),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "telemetry-batch".to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: "2026-08-19T00:00:00Z".to_owned(),
                aoi_name: "Telemetry AOI".to_owned(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]]
                    })
                    .as_object()
                    .expect("polygon")
                    .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "telemetry-batch".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
        })
        .expect("save batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "telemetry-task".to_owned(),
            batch_id: "telemetry-batch".to_owned(),
            selected_item_ids: vec!["ITEM-A".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 100,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-19T00:00:01Z".to_owned(),
        })
        .expect("create task");
}

fn frozen_access(item_id: &str, expected_size_bytes: u64) -> FrozenAssetAccess {
    serde_json::from_value(json!({
        "reference": {
            "provider_id": "asf.nasa.earthdata",
            "collection_id": "sar.nisar.l.rslc.provisional.v1",
            "provider_product_id": item_id,
            "provider_product_key": item_id,
            "canonical_product_key": null,
            "asset_key": "primary",
            "stable_asset_key": format!("{item_id}:primary")
        },
        "descriptor": {
            "resolver_id": "asf-earthdata",
            "scheme": "earthdata_https",
            "stable_href": format!("https://example.invalid/{item_id}.h5"),
            "requires_credentials": true,
            "credential_profile": "earthdata",
            "resolver_parameters": {}
        },
        "role": "primary",
        "file_name": format!("{item_id}.h5"),
        "expected_size_bytes": expected_size_bytes,
        "checksum_algorithm": null,
        "checksum_value": null
    }))
    .expect("valid frozen asset access")
}

fn seed_three_source_task(store: &mut TaskStore, output_root: &Path) {
    seed_three_source_task_with_sizes(
        store,
        output_root,
        [
            ("NISAR-ONE", Some(14_252_244_992_u64), Some(json!(7))),
            ("NISAR-TWO", None, Some(json!(14_252_244_993_u64))),
            ("NISAR-THREE", Some(14_252_244_994_u64), None),
        ],
    );
}

fn seed_three_source_task_with_sizes(
    store: &mut TaskStore,
    output_root: &Path,
    sizes: [(&str, Option<u64>, Option<serde_json::Value>); 3],
) {
    let candidates = sizes
        .into_iter()
        .map(|(item_id, asset_size, fallback_size)| SnapshotItem {
            item_id: item_id.to_owned(),
            provider_id: "asf.nasa.earthdata".to_owned(),
            display_name: format!("{item_id}.h5"),
            acquisition_datetime: Some("2026-08-19T00:00:00Z".to_owned()),
            footprint_geojson: None,
            metadata: Map::new(),
            payload: fallback_size
                .map(|value| Map::from_iter([("file_size_remote".to_owned(), value)]))
                .unwrap_or_default(),
            asset_access: asset_size.map(|value| frozen_access(item_id, value)),
            execution_policy: None,
        })
        .collect::<Vec<_>>();
    let selected_item_ids = candidates
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<Vec<_>>();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "three-source-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-08-19T00:00:00Z".to_owned(),
                aoi_name: "Three source AOI".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: "three-source-batch".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
        })
        .expect("save three-source batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "three-source-task".to_owned(),
            batch_id: "three-source-batch".to_owned(),
            selected_item_ids,
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 100,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-19T00:00:01Z".to_owned(),
        })
        .expect("create three-source task");
}

fn projected_source_total(store: &TaskStore, task_id: &str) -> Option<u64> {
    let task = store
        .core_restore(false)
        .expect("restore")
        .tasks
        .into_iter()
        .find(|task| task.task_id == task_id)
        .expect("task summary");
    serde_json::to_value(task)
        .expect("serialize summary")
        .get("source_total_bytes")
        .and_then(serde_json::Value::as_u64)
}

fn task_page_json(
    store: &TaskStore,
    task_id: &str,
    expected_revision: u64,
    page_index: u64,
    page_size: u64,
) -> serde_json::Value {
    serde_json::to_value(
        store
            .load_task_page(&StoreTaskPageRequest {
                task_id: task_id.to_owned(),
                expected_revision,
                page_index,
                page_size,
            })
            .expect("load bounded task page"),
    )
    .expect("serialize bounded task page")
}

#[test]
fn archived_task_pages_keep_frozen_source_and_persisted_network_byte_facts() {
    let database = TempDatabase::new("archived-task-page-byte-facts");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_three_source_task(&mut store, &output_root);
    store
        .save_task_transfer_telemetry_with_network(
            &StoreTaskTransferTelemetryUpdate {
                task_id: "three-source-task".to_owned(),
                expected_revision: 0,
                transfer_active_millis: 196_095,
                updated_at: "2026-08-19T00:00:02Z".to_owned(),
            },
            Some(282_034_555),
        )
        .expect("save measured network bytes");

    let outputs = [
        ("NISAR-ONE", 187_677_352_u64),
        ("NISAR-TWO", 100_488_907_u64),
        ("NISAR-THREE", 91_000_003_u64),
    ];
    for (revision, (item_id, output_bytes)) in outputs.into_iter().enumerate() {
        store
            .save_download_result(
                &DownloadItemResult {
                    task_id: "three-source-task".to_owned(),
                    item_id: item_id.to_owned(),
                    outcome: DownloadOutcome::Completed,
                    destination: output_root.join(format!("{item_id}.h5")),
                    bytes_written: output_bytes,
                    attempt_count: 1,
                    failure: None,
                    finished_at: "2026-08-19T00:00:10Z".to_owned(),
                },
                revision as u64,
            )
            .expect("save completed output");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "three-source-task".to_owned(),
                expected_revision: revision as u64,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec![item_id.to_owned()],
                },
                event_id: format!("task-page-complete-{item_id}"),
                occurred_at: "2026-08-19T00:00:11Z".to_owned(),
            })
            .expect("complete selected item");
    }
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "three-source-task".to_owned(),
            expected_revision: 3,
            archived_at: "2026-08-19T00:00:12Z".to_owned(),
        })
        .expect("archive completed task");
    drop(store);

    let reopened = TaskStore::open(database.path()).expect("reopen archived store");
    let first = task_page_json(&reopened, "three-source-task", 3, 0, 2);
    let second = task_page_json(&reopened, "three-source-task", 3, 1, 2);
    const SOURCE_TOTAL: u64 = 42_756_734_979;
    for page in [&first, &second] {
        assert_eq!(page["total_count"], json!(3));
        assert_eq!(page["source_total_bytes"], json!(SOURCE_TOTAL));
        assert_eq!(page["network_bytes_received"], json!(282_034_555));
    }
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(second["items"].as_array().unwrap().len(), 1);

    let item_facts = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|item| {
            (
                item["item"]["item_id"].as_str().unwrap(),
                (
                    item["source_total_bytes"].as_u64(),
                    item["total_bytes"].as_u64(),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        item_facts,
        std::collections::BTreeMap::from([
            ("NISAR-ONE", (Some(14_252_244_992), Some(187_677_352))),
            ("NISAR-THREE", (Some(14_252_244_994), Some(91_000_003))),
            ("NISAR-TWO", (Some(14_252_244_993), Some(100_488_907))),
        ])
    );
}

#[test]
fn bounded_task_page_distinguishes_unknown_from_observed_zero_network_bytes() {
    for (label, measured, expected) in [
        ("network-unknown", None, serde_json::Value::Null),
        ("network-observed-zero", Some(0), json!(0)),
    ] {
        let database = TempDatabase::new(label);
        let output_root = database.path().with_extension("output");
        let mut store = TaskStore::open(database.path()).expect("open store");
        seed_task(&mut store, &output_root);
        if let Some(bytes) = measured {
            store
                .save_task_transfer_telemetry_with_network(
                    &StoreTaskTransferTelemetryUpdate {
                        task_id: "telemetry-task".to_owned(),
                        expected_revision: 0,
                        transfer_active_millis: 0,
                        updated_at: "2026-08-19T00:00:02Z".to_owned(),
                    },
                    Some(bytes),
                )
                .expect("save observed network sample");
        }
        let page = task_page_json(&store, "telemetry-task", 0, 0, 1);
        assert_eq!(page["network_bytes_received"], expected, "{label}");
    }
}

#[test]
fn bounded_task_page_source_bytes_fail_closed_per_frozen_item_and_checked_sum() {
    for (label, sizes, expected_items) in [
        (
            "task-page-source-missing",
            [
                ("NISAR-ONE", Some(10_u64), None),
                ("NISAR-TWO", None, None),
                ("NISAR-THREE", Some(30_u64), None),
            ],
            [Some(10_u64), None, Some(30_u64)],
        ),
        (
            "task-page-source-invalid",
            [
                ("NISAR-ONE", Some(10_u64), None),
                ("NISAR-TWO", None, Some(json!(-1))),
                ("NISAR-THREE", Some(30_u64), None),
            ],
            [Some(10_u64), None, Some(30_u64)],
        ),
        (
            "task-page-source-overflow",
            [
                ("NISAR-ONE", Some(i64::MAX as u64), None),
                ("NISAR-TWO", Some(i64::MAX as u64), None),
                ("NISAR-THREE", Some(2_u64), None),
            ],
            [Some(i64::MAX as u64), Some(i64::MAX as u64), Some(2_u64)],
        ),
    ] {
        let database = TempDatabase::new(label);
        let output_root = database.path().with_extension("output");
        let mut store = TaskStore::open(database.path()).expect("open store");
        seed_three_source_task_with_sizes(&mut store, &output_root, sizes);
        let page = task_page_json(&store, "three-source-task", 0, 0, 3);
        assert_eq!(
            page["source_total_bytes"],
            serde_json::Value::Null,
            "{label} task sum"
        );
        assert_eq!(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["source_total_bytes"].as_u64())
                .collect::<Vec<_>>(),
            expected_items,
            "{label} item facts"
        );
    }
}

#[test]
fn frozen_source_total_is_constant_across_completion_pause_resume_and_reopen() {
    let database = TempDatabase::new("frozen-source-total");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_three_source_task(&mut store, &output_root);
    const SOURCE_TOTAL: u64 = 42_756_734_979;
    assert_eq!(
        projected_source_total(&store, "three-source-task"),
        Some(SOURCE_TOTAL)
    );
    assert_eq!(
        store.core_restore(false).unwrap().tasks[0].total_bytes,
        Some(SOURCE_TOTAL)
    );

    for (download_revision, lifecycle_revision, item_id, output_bytes) in [
        (0, 0, "NISAR-ONE", 187_677_352),
        (1, 3, "NISAR-TWO", 100_488_907),
        (2, 4, "NISAR-THREE", 91_000_003),
    ] {
        store
            .save_download_result(
                &DownloadItemResult {
                    task_id: "three-source-task".to_owned(),
                    item_id: item_id.to_owned(),
                    outcome: DownloadOutcome::Completed,
                    destination: output_root.join(format!("{item_id}.h5")),
                    bytes_written: output_bytes,
                    attempt_count: 1,
                    failure: None,
                    finished_at: "2026-08-19T00:00:10Z".to_owned(),
                },
                download_revision,
            )
            .expect("save output result");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "three-source-task".to_owned(),
                expected_revision: lifecycle_revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec![item_id.to_owned()],
                },
                event_id: format!("complete-{item_id}"),
                occurred_at: "2026-08-19T00:00:11Z".to_owned(),
            })
            .expect("complete item");
        assert_eq!(
            projected_source_total(&store, "three-source-task"),
            Some(SOURCE_TOTAL)
        );

        if item_id == "NISAR-ONE" {
            let mixed = store
                .core_restore(false)
                .unwrap()
                .tasks
                .remove(0)
                .total_bytes;
            assert_eq!(
                mixed,
                Some(28_692_167_339),
                "generic total keeps output/source semantics"
            );
            store
                .update_task(&StoreUpdateTaskRequest {
                    task_id: "three-source-task".to_owned(),
                    expected_revision: 1,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::PauseTask,
                        item_ids: vec![],
                    },
                    event_id: "pause-three-source".to_owned(),
                    occurred_at: "2026-08-19T00:00:12Z".to_owned(),
                })
                .expect("pause task");
            store
                .update_task(&StoreUpdateTaskRequest {
                    task_id: "three-source-task".to_owned(),
                    expected_revision: 2,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::ResumeTask,
                        item_ids: vec![],
                    },
                    event_id: "resume-three-source".to_owned(),
                    occurred_at: "2026-08-19T00:00:13Z".to_owned(),
                })
                .expect("resume task");
        }
    }
    assert_eq!(
        store.core_restore(false).unwrap().tasks[0].total_bytes,
        Some(379_166_262)
    );
    drop(store);
    let reopened = TaskStore::open(database.path()).expect("reopen store");
    assert_eq!(
        projected_source_total(&reopened, "three-source-task"),
        Some(SOURCE_TOTAL)
    );
}

#[test]
fn frozen_source_total_fails_closed_for_missing_and_invalid_sizes() {
    for (label, size, expected) in [
        ("zero", Some(json!(0)), Some(0)),
        ("missing", None, None),
        ("negative", Some(json!(-1)), None),
        ("fraction", Some(json!(1.5)), None),
        ("string", Some(json!("1000")), None),
    ] {
        let database = TempDatabase::new(label);
        let mut store = TaskStore::open(database.path()).unwrap();
        seed_task_with_size(&mut store, &database.path().with_extension("output"), size);
        assert_eq!(
            projected_source_total(&store, "telemetry-task"),
            expected,
            "{label}"
        );
    }
}

#[test]
fn schema_v15_creates_fixed_task_transfer_telemetry_authority() {
    let database = TempDatabase::new("schema");
    TaskStore::open(database.path()).expect("open store");

    let connection = Connection::open(database.path()).expect("open sqlite");
    let columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('task_transfer_telemetry')",
            [],
            |row| row.get(0),
        )
        .expect("telemetry columns");
    assert_eq!(columns, 4);
    let history_column: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('task_history_summary') WHERE name = 'transfer_active_millis'",
            [],
            |row| row.get(0),
        )
        .expect("history telemetry column");
    assert_eq!(history_column, 1);
}

#[test]
fn frozen_archive_sizes_populate_queued_and_paused_task_totals_after_reopen() {
    let database = TempDatabase::new("frozen-archive-size");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task_with_size(
        &mut store,
        &database.path().with_extension("output"),
        Some(json!(1000)),
    );
    let request = StoreTaskPageRequest {
        task_id: "telemetry-task".to_owned(),
        expected_revision: 0,
        page_index: 0,
        page_size: 100,
    };
    let task = store.core_restore(false).unwrap().tasks.remove(0);
    assert_eq!(
        task.total_bytes,
        Some(1000),
        "ASF's frozen catalogue bytes are already known before transfer"
    );
    assert_eq!(task.transferred_bytes, 0);
    assert_eq!(
        store.load_task_page(&request).unwrap().items[0].total_bytes,
        Some(1000)
    );
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "telemetry-task".to_owned(),
                item_id: "ITEM-A".to_owned(),
                status: DownloadItemStatus::Paused,
                bytes_completed: 250,
                total_bytes: None,
                bytes_per_second: 0.0,
                attempt: 1,
                updated_at: "2026-08-19T00:00:02Z".to_owned(),
            },
            0,
        )
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "frozen-size-pause".to_owned(),
            occurred_at: "2026-08-19T00:00:03Z".to_owned(),
        })
        .unwrap();
    drop(store);
    let reopened = TaskStore::open(database.path()).unwrap();
    let task = reopened.core_restore(false).unwrap().tasks.remove(0);
    assert_eq!(task.total_bytes, Some(1000));
    assert_eq!(task.transferred_bytes, 250);
    assert_eq!(
        reopened.load_task_page(&request).unwrap().items[0].total_bytes,
        Some(1000)
    );
}

#[test]
fn invalid_frozen_archive_sizes_remain_unknown() {
    for value in [json!(null), json!(-1), json!(1.5), json!("1000")] {
        let database = TempDatabase::new("invalid-frozen-size");
        let mut store = TaskStore::open(database.path()).unwrap();
        seed_task_with_size(
            &mut store,
            &database.path().with_extension("output"),
            Some(value),
        );
        assert_eq!(
            store.core_restore(false).unwrap().tasks[0].total_bytes,
            None
        );
    }
}

#[test]
fn measured_remote_total_overrides_frozen_archive_size_without_losing_actual_progress() {
    let database = TempDatabase::new("measured-total");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task_with_size(
        &mut store,
        &database.path().with_extension("output"),
        Some(json!(1000)),
    );
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "telemetry-task".to_owned(),
                item_id: "ITEM-A".to_owned(),
                status: DownloadItemStatus::Paused,
                bytes_completed: 250,
                total_bytes: Some(1200),
                bytes_per_second: 0.0,
                attempt: 1,
                updated_at: "2026-08-19T00:00:02Z".to_owned(),
            },
            0,
        )
        .unwrap();
    let task = store.core_restore(false).unwrap().tasks.remove(0);
    assert_eq!(task.total_bytes, Some(1200));
    assert_eq!(task.transferred_bytes, 250);
}

#[test]
fn transfer_elapsed_is_monotonic_and_survives_reopen() {
    let database = TempDatabase::new("monotonic");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);

    for (millis, updated_at) in [
        (1_000, "2026-08-19T00:00:02Z"),
        (2_500, "2026-08-19T00:00:03Z"),
    ] {
        store
            .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
                task_id: "telemetry-task".to_owned(),
                expected_revision: 0,
                transfer_active_millis: millis,
                updated_at: updated_at.to_owned(),
            })
            .expect("save monotonic telemetry");
    }

    let regression = store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            transfer_active_millis: 2_000,
            updated_at: "2026-08-19T00:00:04Z".to_owned(),
        })
        .expect_err("elapsed time cannot move backwards");
    assert_eq!(regression.code(), "V3_STORE_INVALID_SNAPSHOT");
    drop(store);

    let reopened = TaskStore::open(database.path()).expect("reopen store");
    let task = reopened
        .core_restore(false)
        .expect("restore")
        .tasks
        .into_iter()
        .find(|task| task.task_id == "telemetry-task")
        .expect("task summary");
    assert_eq!(task.transfer_active_millis, 2_500);
}

#[test]
fn network_measurement_is_separate_from_outputs_and_survives_restart() {
    let database = TempDatabase::new("network-bytes");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, &database.path().with_extension("output"));
    assert_eq!(
        store.core_restore(false).unwrap().tasks[0].network_bytes_received,
        None
    );
    assert_eq!(
        store.core_restore(false).unwrap().tasks[0].reusable_verified_bytes,
        None
    );
    assert_eq!(
        store.core_restore(false).unwrap().tasks[0].reused_verified_bytes,
        None
    );
    let update = StoreTaskTransferTelemetryUpdate {
        task_id: "telemetry-task".to_owned(),
        expected_revision: 0,
        transfer_active_millis: 1000,
        updated_at: "2026-08-19T00:00:03Z".to_owned(),
    };
    store
        .save_task_transfer_telemetry_with_network(&update, Some(10_000))
        .unwrap();
    assert!(store
        .save_task_transfer_telemetry_with_network(&update, Some(9_000))
        .is_err());
    store.save_task_transfer_telemetry(&update).unwrap(); // older caller cannot erase measurement
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "telemetry-task".into(),
                item_id: "ITEM-A".into(),
                outcome: DownloadOutcome::Completed,
                destination: database.path().with_extension("output"),
                bytes_written: 42,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-19T00:00:04Z".into(),
            },
            0,
        )
        .unwrap();
    drop(store);
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened
            .task_transfer_checkpoint_baseline("telemetry-task")
            .unwrap(),
        (1000, 10_000)
    );
    assert_eq!(
        reopened.core_restore(false).unwrap().tasks[0].network_bytes_received,
        Some(10_000)
    );
    assert_eq!(
        reopened
            .load_download_snapshot("telemetry-task")
            .unwrap()
            .results[0]
            .bytes_written,
        42
    );
}

#[test]
fn schema_v26_preserves_v25_rows_without_inventing_network_bytes() {
    let database = TempDatabase::new("network-migration");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, &database.path().with_extension("output"));
    store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "telemetry-task".into(),
            expected_revision: 0,
            transfer_active_millis: 1234,
            updated_at: "2026-08-19T00:00:03Z".into(),
        })
        .unwrap();
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    connection.execute_batch("ALTER TABLE task_transfer_telemetry DROP COLUMN network_bytes_received; DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2; DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers; PRAGMA user_version=25;").unwrap();
    drop(connection);
    let reopened = TaskStore::open(database.path()).unwrap();
    let task = reopened.core_restore(false).unwrap().tasks.remove(0);
    assert_eq!(task.transfer_active_millis, 1234);
    assert_eq!(task.network_bytes_received, None);
}

#[test]
fn schema_v26_rejects_a_preexisting_unchecked_network_column() {
    let database = TempDatabase::new("unsafe-network-migration");
    drop(TaskStore::open(database.path()).unwrap());
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE task_transfer_telemetry DROP COLUMN network_bytes_received;
        ALTER TABLE task_transfer_telemetry ADD COLUMN network_bytes_received INTEGER;
        PRAGMA user_version=25;",
        )
        .unwrap();
    assert!(TaskStore::open(database.path()).is_err());
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(
        version, 25,
        "unsafe migration must not promote Store schema"
    );
}

#[test]
fn transfer_checkpoint_cas_uses_download_revision_not_lifecycle_revision() {
    let database = TempDatabase::new("download-revision-authority");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "diverge-lifecycle-revision".to_owned(),
            occurred_at: "2026-08-19T00:00:02Z".to_owned(),
        })
        .expect("advance only lifecycle revision");
    store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            transfer_active_millis: 100,
            updated_at: "2026-08-19T00:00:03Z".to_owned(),
        })
        .expect("lifecycle divergence must not reject current download revision");

    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "telemetry-task".to_owned(),
                item_id: "ITEM-A".to_owned(),
                status: DownloadItemStatus::Paused,
                bytes_completed: 1,
                total_bytes: Some(10),
                bytes_per_second: 0.0,
                attempt: 1,
                updated_at: "2026-08-19T00:00:04Z".to_owned(),
            },
            0,
        )
        .expect("advance download snapshot revision");
    let stale = store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            transfer_active_millis: 200,
            updated_at: "2026-08-19T00:00:05Z".to_owned(),
        })
        .expect_err("stale download snapshot revision must be rejected");
    assert_eq!(stale.code(), "V3_STORE_REVISION_CONFLICT");

    store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
            transfer_active_millis: 200,
            updated_at: "2026-08-19T00:00:06Z".to_owned(),
        })
        .expect("current download snapshot revision");
    let rollback = store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
            transfer_active_millis: 150,
            updated_at: "2026-08-19T00:00:07Z".to_owned(),
        })
        .expect_err("current revision must still reject elapsed-time rollback");
    assert_eq!(rollback.code(), "V3_STORE_INVALID_SNAPSHOT");
}

#[test]
fn transfer_elapsed_is_materialized_into_bounded_history() {
    let database = TempDatabase::new("history");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            transfer_active_millis: 3_210,
            updated_at: "2026-08-19T00:00:02Z".to_owned(),
        })
        .expect("save telemetry");
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "telemetry-task".to_owned(),
                item_id: "ITEM-A".to_owned(),
                outcome: DownloadOutcome::Completed,
                destination: output_root.join("ITEM-A.zip"),
                bytes_written: 868_220_928,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-19T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save completed bytes");
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["ITEM-A".to_owned()],
            },
            event_id: "telemetry-complete".to_owned(),
            occurred_at: "2026-08-19T00:00:03Z".to_owned(),
        })
        .expect("complete task");
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
            archived_at: "2026-08-19T00:00:04Z".to_owned(),
        })
        .expect("archive task");
    drop(store);

    let reopened = TaskStore::open(database.path()).expect("reopen store");
    let history = reopened
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            after_archived_at_epoch_ms: None,
            after_task_id: None,
            page_size: 40,
        })
        .expect("history page");
    assert_eq!(history.items.len(), 1);
    assert_eq!(history.items[0].task.transfer_active_millis, 3_210);
    assert_eq!(history.items[0].task.transferred_bytes, 868_220_928);
    assert_eq!(history.items[0].task.terminal_bytes, 868_220_928);
    assert_eq!(history.items[0].task.total_bytes, Some(868_220_928));
    assert_eq!(history.items[0].task.reusable_verified_bytes, None);
    assert_eq!(history.items[0].task.reused_verified_bytes, None);
    assert_eq!(
        serde_json::to_value(&history.items[0].task).unwrap()["source_total_bytes"],
        serde_json::Value::Null,
        "archive-only summaries do not invent frozen source bytes"
    );
}

#[test]
fn task_event_page_is_revisioned_bounded_and_never_projects_raw_json() {
    let database = TempDatabase::new("events");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    drop(store);

    let connection = Connection::open(database.path()).expect("open raw sqlite");
    for (sequence, event_type, item_id) in [
        (7, "mark_active", "ITEM-A"),
        (8, "pause", "ITEM-A"),
        (9, "resume", "ITEM-A"),
    ] {
        connection
            .execute(
                "INSERT INTO task_events(event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json)
                 VALUES (?1, 'telemetry-task', ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    format!("event-{sequence}"),
                    sequence,
                    format!("2026-08-19T00:00:{sequence:02}Z"),
                    event_type,
                    item_id,
                    r#"{"password":"must-never-leave-store","raw":"unbounded"}"#,
                    r#"{"code":"HTTP_TIMEOUT","credential":"must-never-leave-store"}"#,
                ],
            )
            .expect("insert event fixture");
    }
    drop(connection);

    let store = TaskStore::open(database.path()).expect("reopen store");
    let first = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            before_sequence: None,
            page_size: 2,
        })
        .expect("first event page");
    assert!(first.has_more);
    assert_eq!(first.next_before_sequence, Some(8));
    assert_eq!(
        first
            .items
            .iter()
            .map(|item| (
                item.sequence,
                item.event_kind.as_str(),
                item.message_zh.as_str()
            ))
            .collect::<Vec<_>>(),
        [(9, "resumed", "下载已继续"), (8, "paused", "下载已暂停")]
    );
    assert!(first.items.iter().all(|item| {
        item.item_label.as_deref() == Some("ITEM-A") && item.code.as_deref() == Some("HTTP_TIMEOUT")
    }));
    let serialized = serde_json::to_string(&first).expect("serialize bounded event page");
    assert!(!serialized.contains("password"));
    assert!(!serialized.contains("credential"));
    assert!(!serialized.contains("unbounded"));

    let second = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            before_sequence: first.next_before_sequence,
            page_size: 2,
        })
        .expect("second event page");
    assert!(!second.has_more);
    assert_eq!(second.items.len(), 2);
    assert_eq!(second.items[0].event_kind, "started");

    let stale_caller_revision = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
            before_sequence: None,
            page_size: 2,
        })
        .expect("read-only event page observes the current revision");
    assert_eq!(stale_caller_revision.revision, 0);
    assert_eq!(stale_caller_revision.items, first.items);
}

#[test]
fn output_directory_resolution_is_exact_for_current_and_archived_tasks() {
    let database = TempDatabase::new("output-directory");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);

    let current = store
        .resolve_task_output_directory(&StoreTaskOutputDirectoryRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
        })
        .expect("current output directory");
    assert_eq!(current.output_root, output_root);
    let stale = store
        .resolve_task_output_directory(&StoreTaskOutputDirectoryRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
        })
        .expect_err("stale output directory request");
    assert_eq!(stale.code(), "V3_STORE_REVISION_CONFLICT");

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["ITEM-A".to_owned()],
            },
            event_id: "output-complete".to_owned(),
            occurred_at: "2026-08-19T00:00:03Z".to_owned(),
        })
        .expect("complete task");
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
            archived_at: "2026-08-19T00:00:04Z".to_owned(),
        })
        .expect("archive task");
    drop(store);

    let reopened = TaskStore::open(database.path()).expect("reopen store");
    let archived = reopened
        .resolve_task_output_directory(&StoreTaskOutputDirectoryRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
        })
        .expect("archived output directory");
    assert_eq!(archived.output_root, output_root);
}

#[test]
fn production_failure_event_projects_only_the_sanitized_result_code() {
    let database = TempDatabase::new("failure-event");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "telemetry-task".to_owned(),
                item_id: "ITEM-A".to_owned(),
                outcome: DownloadOutcome::Failed,
                destination: output_root.join("ITEM-A.zip"),
                bytes_written: 128,
                attempt_count: 2,
                failure: Some(DownloadFailure {
                    code: "HTTP_TIMEOUT".to_owned(),
                    reason_zh: "已清理的超时说明".to_owned(),
                    retryable: true,
                }),
                finished_at: "2026-08-19T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save failed result");
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec!["ITEM-A".to_owned()],
            },
            event_id: "failure-event".to_owned(),
            occurred_at: "2026-08-19T00:00:03Z".to_owned(),
        })
        .expect("mark failed");

    let page = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 1,
            before_sequence: None,
            page_size: 10,
        })
        .expect("failure event page");
    let failure = page
        .items
        .iter()
        .find(|item| item.event_kind == "failed")
        .expect("failed event");
    assert_eq!(failure.code.as_deref(), Some("HTTP_TIMEOUT"));
    let serialized = serde_json::to_string(failure).expect("serialize event");
    assert!(!serialized.contains("已清理的超时说明"));
    assert!(!serialized.contains("ITEM-A.zip"));
}

#[test]
fn production_task_commands_project_useful_localized_events_and_product_labels() {
    let database = TempDatabase::new("production-command-events");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, &output_root);
    drop(store);

    let connection = Connection::open(database.path()).expect("open raw sqlite");
    let events = [
        (10, "task_created", None),
        (
            11,
            "activate_items",
            Some("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1A_USEFUL_PRODUCT"),
        ),
        (12, "pause_task", None),
        (13, "resume_task", None),
        (14, "cancel_task", None),
    ];
    for (sequence, event_type, item_id) in events {
        connection
            .execute(
                "INSERT INTO task_events(event_id, task_id, sequence, occurred_at, event_type, item_id, payload_json, error_json)
                 VALUES (?1, 'telemetry-task', ?2, ?3, ?4, ?5, '{}', NULL)",
                rusqlite::params![
                    format!("production-event-{sequence}"),
                    sequence,
                    format!("2026-08-19T00:01:{sequence:02}Z"),
                    event_type,
                    item_id,
                ],
            )
            .expect("insert production event fixture");
    }
    drop(connection);

    let store = TaskStore::open(database.path()).expect("reopen store");
    let page = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "telemetry-task".to_owned(),
            expected_revision: 0,
            before_sequence: None,
            page_size: 10,
        })
        .expect("load event page");
    let by_sequence = page
        .items
        .iter()
        .map(|item| (item.sequence, item))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(by_sequence[&10].message_zh, "任务已创建");
    assert_eq!(by_sequence[&11].message_zh, "文件开始下载");
    assert_eq!(
        by_sequence[&11].item_label.as_deref(),
        Some("S1A_USEFUL_PRODUCT")
    );
    assert_eq!(by_sequence[&12].message_zh, "任务已暂停");
    assert_eq!(by_sequence[&13].message_zh, "任务已继续");
    assert_eq!(by_sequence[&14].message_zh, "任务已结束");
    assert!(page.items.iter().all(|item| item.event_kind != "updated"));
}

fn three_source_summary(store: &TaskStore) -> insar_store::StoreTaskSummary {
    store
        .core_restore(true)
        .expect("restore")
        .tasks
        .into_iter()
        .find(|task| task.task_id == "three-source-task")
        .expect("three-source task")
}

/// Review 2026-10-17 round 2, P2-2: active items are grouped by what their
/// current progress row reports (none yet in this run, a positive rate, a zero
/// rate). The zero-rate row a NISAR subset worker publishes at the end of its
/// science transfer carries no total, so it sets no persisted total: the row
/// keeps no total and a later Completed result keeps the total its own facts
/// give (here bytes_written, as no total was ever reported).
#[test]
fn core_restore_groups_active_items_by_progress_and_a_phase_row_sets_no_total() {
    let database = TempDatabase::new("active-progress");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_three_source_task(&mut store, &output_root);
    store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: "three-source-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["NISAR-ONE".to_owned(), "NISAR-TWO".to_owned()],
            },
            event_id: "activate-two".to_owned(),
            occurred_at: "2026-08-19T00:00:02Z".to_owned(),
        })
        .expect("activate");
    assert_eq!(
        three_source_summary(&store).active_progress,
        StoreTaskActiveProgressSummary {
            without_progress: 2,
            transferring: 0,
            idle: 0,
            waiting: 0
        }
    );
    let progress = |item_id: &str, bytes_completed, bytes_per_second| DownloadItemProgress {
        task_id: "three-source-task".to_owned(),
        item_id: item_id.to_owned(),
        status: DownloadItemStatus::Active,
        bytes_completed,
        total_bytes: None,
        bytes_per_second,
        attempt: 1,
        updated_at: "2026-08-19T00:00:03Z".to_owned(),
    };
    store
        .save_scheduler_download_progress(&progress("NISAR-ONE", 4_096, 512.0))
        .expect("transferring row");
    store
        .save_scheduler_download_progress(&progress("NISAR-TWO", 2_048, 0.0))
        .expect("idle row");
    assert_eq!(
        three_source_summary(&store).active_progress,
        StoreTaskActiveProgressSummary {
            without_progress: 0,
            transferring: 1,
            idle: 1,
            waiting: 0
        }
    );
    let phase_row_total: Option<i64> = Connection::open(database.path())
        .expect("open raw")
        .query_row(
            "SELECT last_known_total_bytes FROM download_progress \
             WHERE task_id='three-source-task' AND item_id='NISAR-TWO'",
            [],
            |row| row.get(0),
        )
        .expect("phase row");
    assert_eq!(phase_row_total, None);
    store
        .save_scheduler_download_result(&DownloadItemResult {
            task_id: "three-source-task".to_owned(),
            item_id: "NISAR-TWO".to_owned(),
            outcome: DownloadOutcome::Completed,
            destination: output_root.join("NISAR-TWO-subset.h5"),
            bytes_written: 9_999,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-19T00:00:04Z".to_owned(),
        })
        .expect("completed result");
    let total: Option<i64> = Connection::open(database.path())
        .expect("open raw")
        .query_row(
            "SELECT last_known_total_bytes FROM download_results \
             WHERE task_id='three-source-task' AND item_id='NISAR-TWO'",
            [],
            |row| row.get(0),
        )
        .expect("result total");
    assert_eq!(total, Some(9_999));
}

/// Downloader enhancement D1 (2026-10-18): an active item whose current row
/// reports `queued` waits for a local resource (a NISAR subset item waiting
/// for the output writer) and transfers nothing. It is a waiting item, not an
/// item producing its output, so the task card can show 等待写出.
#[test]
fn core_restore_counts_a_queued_row_of_an_active_item_as_waiting() {
    let database = TempDatabase::new("active-progress-waiting");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_three_source_task(&mut store, &output_root);
    store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: "three-source-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["NISAR-ONE".to_owned(), "NISAR-TWO".to_owned()],
            },
            event_id: "activate-two".to_owned(),
            occurred_at: "2026-08-19T00:00:02Z".to_owned(),
        })
        .expect("activate");
    let progress = |item_id: &str, status| DownloadItemProgress {
        task_id: "three-source-task".to_owned(),
        item_id: item_id.to_owned(),
        status,
        bytes_completed: 2_048,
        total_bytes: None,
        bytes_per_second: 0.0,
        attempt: 1,
        updated_at: "2026-08-19T00:00:03Z".to_owned(),
    };
    store
        .save_scheduler_download_progress(&progress("NISAR-ONE", DownloadItemStatus::Active))
        .expect("output row");
    store
        .save_scheduler_download_progress(&progress("NISAR-TWO", DownloadItemStatus::Queued))
        .expect("waiting row");
    let summary = three_source_summary(&store).active_progress;
    assert_eq!(
        (summary.without_progress, summary.transferring, summary.idle),
        (0, 0, 1),
        "the waiting item is not counted as producing its output"
    );
    assert_eq!(
        summary,
        StoreTaskActiveProgressSummary {
            without_progress: 0,
            transferring: 0,
            idle: 1,
            waiting: 1
        }
    );
    // Admitted after waiting: the item produces its output again.
    store
        .save_scheduler_download_progress(&progress("NISAR-TWO", DownloadItemStatus::Active))
        .expect("output row after waiting");
    assert_eq!(
        three_source_summary(&store).active_progress,
        StoreTaskActiveProgressSummary {
            without_progress: 0,
            transferring: 0,
            idle: 2,
            waiting: 0
        }
    );
}

/// Review 2026-10-17, L4: the restored task carries the distinct stable
/// failure codes of its failed items and whether each declares a local-only
/// cause (`source=local`), so the task card can leave out login guidance for
/// local failures. Codes only: no reason text crosses the restore boundary.
#[test]
fn core_restore_projects_failed_item_codes_with_local_causes_and_no_reasons() {
    let database = TempDatabase::new("failure-codes");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_three_source_task(&mut store, &output_root);
    for (revision, (item_id, code, reason)) in [
        (
            "NISAR-ONE",
            "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
            "本地校验未通过，本次没有发起任何网络请求",
        ),
        (
            "NISAR-TWO",
            "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED",
            "本机每小时上限 [stage=science_auth_refresh source=local cause=BHOONIDHI_REFRESH_LOCAL_RATE_LIMITED]",
        ),
        ("NISAR-THREE", "old-provider-error", "Original provider response"),
    ]
    .into_iter()
    .enumerate()
    {
        store
            .save_scheduler_download_result(&DownloadItemResult {
                task_id: "three-source-task".to_owned(),
                item_id: item_id.to_owned(),
                outcome: DownloadOutcome::Failed,
                destination: output_root.join(format!("{item_id}.h5")),
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: code.to_owned(),
                    reason_zh: reason.to_owned(),
                    retryable: false,
                }),
                finished_at: "2026-08-19T00:00:02Z".to_owned(),
            })
            .expect("failed result");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "three-source-task".to_owned(),
                expected_revision: revision as u64,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec![item_id.to_owned()],
                },
                event_id: format!("fail-{item_id}"),
                occurred_at: "2026-08-19T00:00:03Z".to_owned(),
            })
            .expect("mark failed");
    }
    let task = three_source_summary(&store);
    assert_eq!(
        task.failure_codes,
        vec![
            StoreTaskFailureCode {
                code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED".to_owned(),
                local: true,
                tls_certificate: false,
                network: false
            },
            StoreTaskFailureCode {
                code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED".to_owned(),
                local: false,
                tls_certificate: false,
                network: false
            },
            StoreTaskFailureCode {
                code: "UNKNOWN_FAILURE_CODE".to_owned(),
                local: false,
                tls_certificate: false,
                network: false
            },
        ]
    );
    let serialized = serde_json::to_string(&task).expect("serialize summary");
    for private in [
        "本地校验未通过",
        "Original provider response",
        "old-provider-error",
        "本机每小时上限",
    ] {
        assert!(!serialized.contains(private), "{private}");
    }
}

/// Downloader enhancement D1d (L4 2026-10-18): a Bhoonidhi download-probe
/// timeout failed as DL005 and the task card asked for a login check. A failure
/// whose diagnostic declares a network transport cause (`source=network`) is
/// projected as such when every failure with its code declares it; the flag is
/// left out when false and no reason text crosses the restore boundary.
#[test]
fn core_restore_projects_a_network_cause_only_when_every_failure_declares_it() {
    let database = TempDatabase::new("failure-codes-network");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_three_source_task(&mut store, &output_root);
    let timeout = "error sending request for url (https://provider.invalid/download) caused by: operation timed out (while sending Bhoonidhi download probe request) [stage=download_probe attempts=3 source=network]";
    for (revision, (item_id, code, reason)) in [
        ("NISAR-ONE", "DL005", timeout),
        ("NISAR-TWO", "DL005", timeout),
        (
            "NISAR-THREE",
            "DL004",
            "Bhoonidhi authentication was rejected",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        store
            .save_scheduler_download_result(&DownloadItemResult {
                task_id: "three-source-task".to_owned(),
                item_id: item_id.to_owned(),
                outcome: DownloadOutcome::Failed,
                destination: output_root.join(format!("{item_id}.h5")),
                bytes_written: 0,
                attempt_count: 3,
                failure: Some(DownloadFailure {
                    code: code.to_owned(),
                    reason_zh: reason.to_owned(),
                    retryable: true,
                }),
                finished_at: "2026-10-19T00:00:02Z".to_owned(),
            })
            .expect("failed result");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "three-source-task".to_owned(),
                expected_revision: revision as u64,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec![item_id.to_owned()],
                },
                event_id: format!("fail-{item_id}"),
                occurred_at: "2026-10-19T00:00:03Z".to_owned(),
            })
            .expect("mark failed");
    }
    let task = three_source_summary(&store);
    assert_eq!(
        serde_json::to_value(&task.failure_codes).expect("serialize codes"),
        serde_json::json!([
            {"code": "DL004", "local": false},
            {"code": "DL005", "local": false, "network": true}
        ])
    );
    let summary = serde_json::to_string(&task).expect("serialize summary");
    assert!(!summary.contains("operation timed out"));
}

/// L5 recheck 2026-10-19: a failure whose diagnostic declares that the server
/// certificate could not be verified (`source=tls_certificate`) is projected
/// as such when every failure with its code declares it, so the task card can
/// name the certificate problem instead of a login or timeout. No reason text
/// crosses the restore boundary, and the flag is left out when false.
#[test]
fn core_restore_projects_a_tls_certificate_cause_only_when_every_failure_declares_it() {
    let database = TempDatabase::new("failure-codes-tls");
    let output_root = database.path().with_extension("output");
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_three_source_task(&mut store, &output_root);
    let tls_reason = "无法验证服务器证书（本机证书库无法读取或网络被拦截），断点已保留。NISAR V2 Range transport failed [stage=preflight transport_cause=tls_certificate source=tls_certificate]";
    for (revision, (item_id, code, reason)) in [
        ("NISAR-ONE", "NISAR_SUBSET_V2_TRANSPORT_FAILED", tls_reason),
        (
            "NISAR-TWO",
            "NISAR_SUBSET_V2_TRANSPORT_FAILED",
            "NISAR V2 Range transport failed [stage=preflight transport_cause=request_timeout]",
        ),
        ("NISAR-THREE", "DL005", tls_reason),
    ]
    .into_iter()
    .enumerate()
    {
        store
            .save_scheduler_download_result(&DownloadItemResult {
                task_id: "three-source-task".to_owned(),
                item_id: item_id.to_owned(),
                outcome: DownloadOutcome::Failed,
                destination: output_root.join(format!("{item_id}.h5")),
                bytes_written: 0,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: code.to_owned(),
                    reason_zh: reason.to_owned(),
                    retryable: true,
                }),
                finished_at: "2026-10-19T00:00:02Z".to_owned(),
            })
            .expect("failed result");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "three-source-task".to_owned(),
                expected_revision: revision as u64,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec![item_id.to_owned()],
                },
                event_id: format!("fail-{item_id}"),
                occurred_at: "2026-10-19T00:00:03Z".to_owned(),
            })
            .expect("mark failed");
    }
    let task = three_source_summary(&store);
    assert_eq!(
        task.failure_codes,
        vec![
            StoreTaskFailureCode {
                code: "DL005".to_owned(),
                local: false,
                tls_certificate: true,
                network: false
            },
            StoreTaskFailureCode {
                code: "NISAR_SUBSET_V2_TRANSPORT_FAILED".to_owned(),
                local: false,
                tls_certificate: false,
                network: false
            },
        ]
    );
    let serialized = serde_json::to_value(&task.failure_codes).expect("serialize codes");
    assert_eq!(
        serialized,
        serde_json::json!([
            {"code": "DL005", "local": false, "tls_certificate": true},
            {"code": "NISAR_SUBSET_V2_TRANSPORT_FAILED", "local": false}
        ])
    );
    let summary = serde_json::to_string(&task).expect("serialize summary");
    assert!(!summary.contains("无法验证服务器证书"));
}
