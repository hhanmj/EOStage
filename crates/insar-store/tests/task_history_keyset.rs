use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreDeleteHistoryRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskOperationDescriptor, TaskStateCommandKind,
};
use insar_store::{StoreTaskHistoryPageRequest, TaskStore};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

const BATCH_ID: &str = "history-keyset-batch";

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-history-keyset-{}-{nonce}-{seq}.sqlite3",
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

fn item(index: usize) -> SnapshotItem {
    SnapshotItem {
        item_id: format!("ITEM-{index:03}"),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("Sentinel-1 {index:03}"),
        acquisition_datetime: Some("2026-08-17T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn oversized_private_item() -> SnapshotItem {
    let mut value = item(999);
    value.item_id = "ITEM-OVERSIZED".to_owned();
    value.display_name = "Valid item with large private metadata".to_owned();
    value.metadata.insert(
        "native_private_payload".to_owned(),
        Value::String("X".repeat(1_500_000)),
    );
    value.payload.insert(
        "native_private_payload".to_owned(),
        Value::String("Y".repeat(1_500_000)),
    );
    value
}

fn create_request(index: usize, output_root: &Path) -> StoreCreateTaskRequest {
    StoreCreateTaskRequest {
        task_id: format!("task-{index:03}"),
        batch_id: BATCH_ID.to_owned(),
        selected_item_ids: vec![format!("ITEM-{index:03}")],
        checked_item_ids: vec![format!("ITEM-{index:03}")],
        page_index: 0,
        page_size: 24,
        output_root: output_root.join(format!("task-{index:03}")),
        layout: DownloadLayout::Sentinel1Slc,
        concurrency: 1,
        created_at: format!("2026-08-16T00:{:02}:00Z", index % 60),
    }
}

fn archive_timestamp(index: usize) -> String {
    if index == 20 || index == 21 {
        "2026-08-17T00:20:00Z".to_owned()
    } else {
        format!("2026-08-17T00:{:02}:00Z", index % 60)
    }
}

fn finish_and_archive(store: &mut TaskStore, index: usize, archived_at: &str) {
    let task_id = format!("task-{index:03}");
    let item_id = format!("ITEM-{index:03}");
    let update = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec![item_id],
            },
            event_id: format!("complete-{index:03}"),
            occurred_at: archived_at.to_owned(),
        })
        .expect("complete task");
    assert_eq!(update.runtime.revision, 1);
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id,
            expected_revision: 1,
            archived_at: archived_at.to_owned(),
        })
        .expect("archive task");
}

fn seed_history(database: &TempDatabase, count: usize) {
    let mut store = TaskStore::open(database.path()).expect("open store");
    let mut candidates = (0..=count).map(item).collect::<Vec<_>>();
    candidates.push(oversized_private_item());
    let first = create_request(0, database.path().parent().expect("parent"));
    store
        .save_search_batch_and_create_task_with_projection(
            &StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: BATCH_ID.to_owned(),
                    provider_id: "asf.sentinel1".to_owned(),
                    created_at: "2026-08-16T00:00:00Z".to_owned(),
                    aoi_name: "History AOI".to_owned(),
                    aoi_geojson: Some(
                        json!({
                            "type": "Polygon",
                            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
                        })
                        .as_object()
                        .expect("AOI")
                        .clone(),
                    ),
                    query: Map::new(),
                    search_stats: None,
                    candidates,
                },
                workbench: SearchWorkbenchState {
                    batch_id: BATCH_ID.to_owned(),
                    checked_item_ids: vec!["ITEM-000".to_owned()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            },
            &first,
            Some(&TaskOperationDescriptor {
                operation_id: "history-operation".to_owned(),
                kind: "dem".to_owned(),
                phase: "download".to_owned(),
                region_id: "region-history".to_owned(),
                approved_worker_operation: "dem_download".to_owned(),
                payload: Map::new(),
            }),
            None,
        )
        .expect("create first projected task");
    finish_and_archive(&mut store, 0, &archive_timestamp(0));
    // A historical empty DEM descriptor remains visible in history without
    // inventing scientific conversion authority from the operation label.
    let inspect =
        Connection::open_with_flags(database.path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let authority_count: i64 = inspect
        .query_row(
            "SELECT COUNT(*) FROM task_history_dem_source_authority WHERE task_id = 'task-000'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(authority_count, 0);
    drop(inspect);
    for index in 1..count {
        store
            .create_task(&create_request(
                index,
                database.path().parent().expect("parent"),
            ))
            .expect("create task");
        finish_and_archive(&mut store, index, &archive_timestamp(index));
    }
}

fn json_has_forbidden_payload(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(json_has_forbidden_payload),
        Value::Object(values) => values.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "snapshot_json"
                    | "state_json"
                    | "candidates"
                    | "selected_item_ids"
                    | "checked_item_ids"
                    | "active_item_ids"
                    | "results"
                    | "progress"
            ) || json_has_forbidden_payload(value)
        }),
        _ => false,
    }
}

#[test]
fn history_keyset_is_stable_bounded_scalar_only_and_skips_degraded_rows() {
    let database = TempDatabase::new();
    seed_history(&database, 50);

    {
        let mut store = TaskStore::open(database.path()).expect("open for tombstone");
        store
            .delete_history(&StoreDeleteHistoryRequest {
                task_id: "task-005".to_owned(),
                deleted_at: "2026-08-17T01:00:00Z".to_owned(),
            })
            .expect("tombstone one history row");
    }

    let connection = Connection::open(database.path()).expect("open raw fixture connection");
    let runtime = |task_id: &str, selected: Value| {
        json!({
            "task_id": task_id,
            "status": "completed",
            "selected_item_ids": selected,
            "checked_item_ids": [],
            "page_index": 0,
            "page_size": 24,
            "active_item_ids": [],
            "paused_item_ids": [],
            "queued_item_ids": [],
            "completed_item_ids": [],
            "failed_item_ids": [],
            "skipped_item_ids": [],
            "cancelled_item_ids": [],
            "task_paused": false,
            "concurrency": 1,
            "revision": 9,
            "updated_at": "2026-08-17T00:10:00Z"
        })
        .to_string()
    };
    let insert_raw = |task_id: &str, archived_at: &str, snapshot: &str, state: &str| {
        connection
            .execute(
                "INSERT INTO task_snapshots(task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json) VALUES (?1, ?2, 'asf.sentinel1', '2026-08-16T00:00:00Z', 'C:\\history\\bounded', 'sentinel1_slc', ?3)",
                params![task_id, BATCH_ID, snapshot],
            )
            .expect("insert raw snapshot");
        connection
            .execute(
                "INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json) VALUES (?1, 'completed', 9, '2026-08-17T00:10:00Z', ?2)",
                params![task_id, state],
            )
            .expect("insert raw runtime");
        connection
            .execute(
                "INSERT INTO task_history(task_id, archived_at, layout, record_json) VALUES (?1, ?2, 'sentinel1_slc', '{}')",
                params![task_id, archived_at],
            )
            .expect("insert raw history");
    };
    let compact_snapshot = |task_id: &str, output_root: &str, operation: Value, item_id: &str| {
        json!({
            "persistence_version": 2,
            "schema_version": 1,
            "task_id": task_id,
            "batch_id": BATCH_ID,
            "provider_id": "asf.sentinel1",
            "created_at": "2026-08-16T00:00:00Z",
            "aoi_name": "History AOI",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
            },
            "query": {},
            "selected_products": [{
                "item_id": item_id,
                "provider_id": "asf.sentinel1",
                "display_name": item_id,
                "acquisition_datetime": "2026-08-16T00:00:00Z",
                "footprint_geojson": null,
                "metadata": {},
                "payload": {},
                "asset_access": null,
                "execution_policy": null
            }],
            "initial_selected_item_ids": [item_id],
            "output_root": output_root,
            "layout": "sentinel1_slc",
            "operation": operation,
            "orbit_association": null,
        })
    };
    let complete_runtime = |task_id: &str,
                            status: &str,
                            revision: u64,
                            updated_at: &str,
                            selected: Value,
                            completed: Value,
                            failed: Value| {
        json!({
            "task_id": task_id,
            "status": status,
            "selected_item_ids": selected,
            "checked_item_ids": [],
            "page_index": 0,
            "page_size": 24,
            "active_item_ids": [],
            "paused_item_ids": [],
            "queued_item_ids": [],
            "completed_item_ids": completed,
            "failed_item_ids": failed,
            "skipped_item_ids": [],
            "cancelled_item_ids": [],
            "task_paused": false,
            "concurrency": 1,
            "revision": revision,
            "updated_at": updated_at,
        })
    };
    let insert_with_columns = |task_id: &str,
                               batch_id: &str,
                               provider_id: &str,
                               output_root: &str,
                               archived_at: &str,
                               status: &str,
                               revision: i64,
                               updated_at: &str,
                               snapshot: &Value,
                               state: &Value| {
        connection
            .execute(
                "INSERT INTO task_snapshots(task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json) VALUES (?1, ?2, ?3, '2026-08-16T00:00:00Z', ?4, 'sentinel1_slc', ?5)",
                params![task_id, batch_id, provider_id, output_root, snapshot.to_string()],
            )
            .expect("insert adversarial snapshot");
        connection
            .execute(
                "INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![task_id, status, revision, updated_at, state.to_string()],
            )
            .expect("insert adversarial runtime");
        connection
            .execute(
                "INSERT INTO task_history(task_id, archived_at, layout, record_json) VALUES (?1, ?2, 'sentinel1_slc', '{}')",
                params![task_id, archived_at],
            )
            .expect("insert adversarial history");
    };
    insert_raw(
        "degraded-malformed-json",
        "2026-08-17T00:58:00Z",
        "not-json",
        &runtime("degraded-malformed-json", json!([])),
    );
    insert_raw(
        "degraded-version",
        "2026-08-17T00:57:00Z",
        &json!({"persistence_version": 999, "schema_version": 1, "operation": null}).to_string(),
        &runtime("degraded-version", json!([])),
    );
    insert_raw(
        "degraded-runtime",
        "2026-08-17T00:56:00Z",
        &json!({"persistence_version": 2, "schema_version": 1, "operation": null}).to_string(),
        &runtime("degraded-runtime", json!("not-an-array")),
    );
    let mut large_snapshot = compact_snapshot(
        "task-oversized",
        "C:\\history\\bounded",
        Value::Null,
        "ITEM-OVERSIZED",
    );
    large_snapshot["selected_products"][0] =
        serde_json::to_value(oversized_private_item()).expect("oversized selected product");
    let large_runtime = complete_runtime(
        "task-oversized",
        "completed",
        9,
        "2026-08-17T00:10:00Z",
        json!(["ITEM-OVERSIZED"]),
        json!(["ITEM-OVERSIZED"]),
        json!([]),
    );
    insert_raw(
        "task-oversized",
        "2026-08-17T00:09:30Z",
        &large_snapshot.to_string(),
        &large_runtime.to_string(),
    );

    let normal_output = "C:\\history\\bounded";
    let normal_runtime = |task_id: &str| {
        complete_runtime(
            task_id,
            "completed",
            9,
            "2026-08-17T00:10:00Z",
            json!(["ITEM-A"]),
            json!(["ITEM-A"]),
            json!([]),
        )
    };
    let adversarial = [
        (
            "bad-snapshot-envelope",
            compact_snapshot("wrong-task", normal_output, Value::Null, "ITEM-A"),
            normal_runtime("bad-snapshot-envelope"),
        ),
        (
            "bad-runtime-status",
            compact_snapshot("bad-runtime-status", normal_output, Value::Null, "ITEM-A"),
            complete_runtime(
                "bad-runtime-status",
                "failed",
                9,
                "2026-08-17T00:10:00Z",
                json!(["ITEM-A"]),
                json!(["ITEM-A"]),
                json!([]),
            ),
        ),
        (
            "bad-runtime-revision",
            compact_snapshot("bad-runtime-revision", normal_output, Value::Null, "ITEM-A"),
            complete_runtime(
                "bad-runtime-revision",
                "completed",
                8,
                "2026-08-17T00:10:00Z",
                json!(["ITEM-A"]),
                json!(["ITEM-A"]),
                json!([]),
            ),
        ),
        (
            "bad-runtime-time",
            compact_snapshot("bad-runtime-time", normal_output, Value::Null, "ITEM-A"),
            complete_runtime(
                "bad-runtime-time",
                "completed",
                9,
                "2026-08-17T00:10:01Z",
                json!(["ITEM-A"]),
                json!(["ITEM-A"]),
                json!([]),
            ),
        ),
        (
            "bad-duplicate-partition",
            compact_snapshot(
                "bad-duplicate-partition",
                normal_output,
                Value::Null,
                "ITEM-A",
            ),
            complete_runtime(
                "bad-duplicate-partition",
                "completed",
                9,
                "2026-08-17T00:10:00Z",
                json!(["ITEM-A", "ITEM-B"]),
                json!(["ITEM-A"]),
                json!(["ITEM-A"]),
            ),
        ),
        (
            "bad-foreign-partition",
            compact_snapshot(
                "bad-foreign-partition",
                normal_output,
                Value::Null,
                "ITEM-A",
            ),
            complete_runtime(
                "bad-foreign-partition",
                "completed",
                9,
                "2026-08-17T00:10:00Z",
                json!(["ITEM-A"]),
                json!(["FOREIGN"]),
                json!([]),
            ),
        ),
        (
            "bad-element-type",
            compact_snapshot("bad-element-type", normal_output, Value::Null, "ITEM-A"),
            complete_runtime(
                "bad-element-type",
                "completed",
                9,
                "2026-08-17T00:10:00Z",
                json!([7]),
                json!([7]),
                json!([]),
            ),
        ),
    ];
    for (index, (task_id, snapshot, state)) in adversarial.into_iter().enumerate() {
        insert_with_columns(
            task_id,
            BATCH_ID,
            "asf.sentinel1",
            normal_output,
            &format!("2026-08-17T00:59:{:02}Z", 50 - index),
            "completed",
            9,
            "2026-08-17T00:10:00Z",
            &snapshot,
            &state,
        );
    }

    let authority_case = |task_id: &str| {
        (
            compact_snapshot(task_id, normal_output, Value::Null, "ITEM-A"),
            normal_runtime(task_id),
        )
    };
    let mut authority_adversarial = Vec::<(&str, Value, Value)>::new();
    let (mut snapshot, runtime) = authority_case("bad-empty-products");
    snapshot["selected_products"] = json!([]);
    snapshot["initial_selected_item_ids"] = json!([]);
    authority_adversarial.push(("bad-empty-products", snapshot, runtime));

    let (mut snapshot, runtime) = authority_case("bad-missing-products");
    snapshot
        .as_object_mut()
        .expect("snapshot object")
        .remove("selected_products");
    authority_adversarial.push(("bad-missing-products", snapshot, runtime));

    let (snapshot, mut runtime) = authority_case("bad-selected-foreign");
    runtime["selected_item_ids"] = json!(["FOREIGN"]);
    runtime["completed_item_ids"] = json!(["FOREIGN"]);
    authority_adversarial.push(("bad-selected-foreign", snapshot, runtime));

    let (snapshot, mut runtime) = authority_case("bad-checked-foreign");
    runtime["checked_item_ids"] = json!(["FOREIGN"]);
    authority_adversarial.push(("bad-checked-foreign", snapshot, runtime));

    let (snapshot, mut runtime) = authority_case("bad-zero-page-size");
    runtime["page_size"] = json!(0);
    authority_adversarial.push(("bad-zero-page-size", snapshot, runtime));

    let (snapshot, mut runtime) = authority_case("bad-zero-concurrency");
    runtime["concurrency"] = json!(0);
    authority_adversarial.push(("bad-zero-concurrency", snapshot, runtime));

    let (mut snapshot, mut runtime) = authority_case("bad-active-concurrency");
    let mut second_product = snapshot["selected_products"][0].clone();
    second_product["item_id"] = json!("ITEM-B");
    second_product["display_name"] = json!("ITEM-B");
    snapshot["selected_products"]
        .as_array_mut()
        .expect("selected product array")
        .push(second_product);
    snapshot["initial_selected_item_ids"] = json!(["ITEM-A", "ITEM-B"]);
    runtime["selected_item_ids"] = json!(["ITEM-A", "ITEM-B"]);
    runtime["active_item_ids"] = json!(["ITEM-A", "ITEM-B"]);
    runtime["completed_item_ids"] = json!([]);
    authority_adversarial.push(("bad-active-concurrency", snapshot, runtime));

    let (mut snapshot, runtime) = authority_case("bad-operation-enum");
    snapshot["operation"] = json!({
        "operation_id": "bad-operation",
        "kind": "unknown",
        "phase": "unknown",
        "region_id": "region-history",
        "approved_worker_operation": "unknown",
        "payload": {}
    });
    authority_adversarial.push(("bad-operation-enum", snapshot, runtime));

    let (mut snapshot, runtime) = authority_case("bad-snapshot-unknown-field");
    snapshot["unknown_top_level"] = json!(true);
    authority_adversarial.push(("bad-snapshot-unknown-field", snapshot, runtime));

    let (snapshot, mut runtime) = authority_case("bad-runtime-unknown-field");
    runtime["unknown_top_level"] = json!(true);
    authority_adversarial.push(("bad-runtime-unknown-field", snapshot, runtime));

    for (index, (task_id, snapshot, state)) in authority_adversarial.into_iter().enumerate() {
        insert_with_columns(
            task_id,
            BATCH_ID,
            "asf.sentinel1",
            normal_output,
            &format!("2026-08-17T01:00:{index:02}Z"),
            "completed",
            9,
            "2026-08-17T00:10:00Z",
            &snapshot,
            &state,
        );
    }
    let huge_operation = "O".repeat(1_200_000);
    insert_with_columns(
        "bad-huge-operation",
        BATCH_ID,
        "asf.sentinel1",
        normal_output,
        "2026-08-17T00:59:59Z",
        "completed",
        9,
        "2026-08-17T00:10:00Z",
        &compact_snapshot(
            "bad-huge-operation",
            normal_output,
            json!({
                "operation_id": huge_operation,
                "kind": "dem",
                "phase": "download",
                "region_id": "region-history",
                "approved_worker_operation": "dem_download"
            }),
            "ITEM-A",
        ),
        &normal_runtime("bad-huge-operation"),
    );
    let huge_output = format!("C:\\{}", "P".repeat(1_200_000));
    insert_with_columns(
        "bad-huge-output",
        BATCH_ID,
        "asf.sentinel1",
        &huge_output,
        "2026-08-17T00:59:58Z",
        "completed",
        9,
        "2026-08-17T00:10:00Z",
        &compact_snapshot("bad-huge-output", &huge_output, Value::Null, "ITEM-A"),
        &normal_runtime("bad-huge-output"),
    );
    insert_with_columns(
        "bad-long-archive-time",
        BATCH_ID,
        "asf.sentinel1",
        normal_output,
        "2026-08-17T00:59:57.1234567890Z",
        "completed",
        9,
        "2026-08-17T00:10:00Z",
        &compact_snapshot(
            "bad-long-archive-time",
            normal_output,
            Value::Null,
            "ITEM-A",
        ),
        &normal_runtime("bad-long-archive-time"),
    );
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS task_history_dem_source_authority;
             DROP TABLE IF EXISTS task_history_summary;
             DROP TABLE IF EXISTS dem_source_identity;
             DROP TABLE IF EXISTS standalone_aoi_state;
             DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE IF EXISTS nisar_archived_recovery_v2;
             DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 8;",
        )
        .expect("prepare v8 history backfill fixture");
    drop(connection);

    let mut expected = (0..50)
        .filter(|index| *index != 5)
        .map(|index| (archive_timestamp(index), format!("task-{index:03}")))
        .collect::<Vec<_>>();
    expected.push((
        "2026-08-17T00:09:30Z".to_owned(),
        "task-oversized".to_owned(),
    ));
    expected.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let mut store = TaskStore::open(database.path()).expect("reopen store");
    let first = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .expect("first history page");
    assert_eq!(first.items.len(), 40, "one exact bounded page");
    assert!(first.has_more, "LIMIT+1 proves a following compatible row");
    let cursor = first.next_cursor.clone().expect("next cursor");
    assert_eq!(cursor.task_id, first.items.last().unwrap().task.task_id);
    assert_eq!(first.items[0].task.revision, 1);
    assert_eq!(first.items[0].task.status, TaskLifecycleStatus::Completed);

    // Insert a newer row after page one. Stable keyset traversal must neither
    // duplicate page-one rows nor hide any row that was originally after it.
    store
        .create_task(&create_request(
            50,
            database.path().parent().expect("parent"),
        ))
        .expect("create inserted task");
    finish_and_archive(&mut store, 50, "2026-08-17T01:50:00Z");

    let second = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: Some(cursor.archived_at_epoch_ms),
            after_task_id: Some(cursor.task_id),
        })
        .expect("second history page");
    assert!(!second.has_more);
    assert!(second.next_cursor.is_none());
    let observed = first
        .items
        .iter()
        .chain(second.items.iter())
        .map(|item| (item.archived_at.clone(), item.task.task_id.clone()))
        .collect::<Vec<_>>();
    assert_eq!(observed, expected);
    assert!(observed.iter().any(|(_, id)| id == "task-oversized"));
    assert_eq!(
        observed.iter().collect::<HashSet<_>>().len(),
        observed.len()
    );
    assert!(!observed.iter().any(|(_, id)| id == "task-050"));
    let tie = observed
        .iter()
        .filter(|(archived_at, _)| archived_at == "2026-08-17T00:20:00Z")
        .map(|(_, task_id)| task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tie, ["task-020", "task-021"]);

    let operation = first
        .items
        .iter()
        .chain(second.items.iter())
        .find(|item| item.task.task_id == "task-000")
        .and_then(|item| item.task.operation.as_ref())
        .expect("operation scalar projection");
    assert_eq!(operation.operation_id, "history-operation");
    assert_eq!(operation.kind, "dem");
    assert_eq!(operation.phase, "download");
    assert_eq!(operation.region_id, "region-history");
    assert_eq!(operation.approved_worker_operation, "dem_download");

    let serialized = serde_json::to_value(&first).expect("serialize page DTO");
    assert!(!json_has_forbidden_payload(&serialized));
    assert!(serialized.get("page_index").is_none());
    assert!(serialized.pointer("/items/0/archived_at").is_some());
    assert!(serialized.pointer("/items/0/revision").is_some());
    assert!(serde_json::to_vec(&serialized).unwrap().len() < 80_000);
}

#[test]
fn history_keyset_rejects_partial_malformed_and_oversized_cursors() {
    let database = TempDatabase::new();
    let store = TaskStore::open(database.path()).expect("open store");
    let invalid = [
        StoreTaskHistoryPageRequest {
            page_size: 0,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        },
        StoreTaskHistoryPageRequest {
            page_size: 101,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        },
        StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: Some(1_787_270_400_000),
            after_task_id: None,
        },
        StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: Some(i64::MIN),
            after_task_id: Some("task-001".to_owned()),
        },
        StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: Some(i64::MAX),
            after_task_id: Some("task-001".to_owned()),
        },
        StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: Some(1_787_270_400_000),
            after_task_id: Some(" task-001".to_owned()),
        },
        StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: Some(1_787_270_400_000),
            after_task_id: Some("x".repeat(129)),
        },
    ];
    for request in invalid {
        let error = store
            .load_task_history_page(&request)
            .expect_err("invalid history cursor must fail closed");
        assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    }
    let exact_boundary = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: Some(0),
            after_task_id: Some("x".repeat(128)),
        })
        .expect("128-character task ID is an accepted cursor scalar");
    assert!(exact_boundary.items.is_empty());
}
