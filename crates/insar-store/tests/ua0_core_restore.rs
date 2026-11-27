use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadItemProgress, DownloadItemStatus, DownloadLayout, FrozenSearchBatch, FrozenSearchStats,
    SearchWorkbenchState, SnapshotItem, StoreAppendTaskRequest, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreDeleteHistoryRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, StoreUpdateTaskViewRequest, TaskOperationDescriptor,
    TaskStateCommandKind, WorkspaceContextState, WorkspaceProjectState, WorkspaceRegionState,
};
use insar_store::{
    LegacyTaskCompatibilityStatus, LegacyTaskRestoreReasonCode, StoreWorkbenchPageRequest,
    TaskStore, STORE_SCHEMA_VERSION,
};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

const CREATED_AT: &str = "2026-08-13T01:00:00Z";
const BATCH_ID: &str = "fixture-batch-10000";
const CURRENT_TASK_ID: &str = "fixture-current-task";
const ADAPTED_TASK_ID: &str = "fixture-legacy-adapted";
const UNAVAILABLE_TASK_ID: &str = "fixture-legacy-unavailable";

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
            "insar-store-ua0-core-{}-{nonce}-{seq}.sqlite3",
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

fn burst_candidate(index: usize) -> SnapshotItem {
    SnapshotItem {
        item_id: format!("BURST-{index:05}"),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("Sanitized Burst {index:05}"),
        acquisition_datetime: Some("2026-08-12T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({
            "dataset": "SENTINEL-1",
            "processing_level": "BURST",
            "product_family": "BURST",
            "product_type": "BURST",
            "provider": "ASF"
        })),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed_current_store(database: &TempDatabase) {
    seed_current_store_with_search_stats(database, Some(10_000));
}

fn seed_current_store_with_search_stats(
    database: &TempDatabase,
    frozen_candidate_count: Option<u64>,
) {
    let mut store = TaskStore::open(database.path()).unwrap();
    let region = WorkspaceRegionState {
        project_id: "project-fixture".to_owned(),
        region_id: "region-fixture".to_owned(),
        region_name: "Fixture Region".to_owned(),
        safe_name: "fixture-region".to_owned(),
    };
    store
        .save_workspace_context(&WorkspaceContextState {
            schema_version: 1,
            workspace_id: "workspace-fixture".to_owned(),
            workspace_root: database.path().parent().unwrap().join("workspace-fixture"),
            workspace_name: "Fixture Workspace".to_owned(),
            project_id: Some("project-fixture".to_owned()),
            project_name: Some("Fixture Project".to_owned()),
            region_id: Some("region-fixture".to_owned()),
            region_name: Some("Fixture Region".to_owned()),
            revision: 7,
            updated_at: CREATED_AT.to_owned(),
            projects: vec![WorkspaceProjectState {
                workspace_id: "workspace-fixture".to_owned(),
                project_id: "project-fixture".to_owned(),
                project_name: "Fixture Project".to_owned(),
                safe_name: "fixture-project".to_owned(),
                regions: vec![region.clone()],
            }],
            regions: vec![region],
        })
        .unwrap();
    store
        .save_region_aoi_for_workspace(
            "workspace-fixture",
            "region-fixture",
            None,
            "drawn",
            &map(json!({
                "type": "Polygon",
                "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
            })),
            &[100.0, 30.0, 101.0, 31.0],
            "EPSG:4326",
            "Fixture AOI",
            CREATED_AT,
            CREATED_AT,
        )
        .unwrap();

    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: BATCH_ID.to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        created_at: CREATED_AT.to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: Some(map(json!({
            "type": "Polygon",
            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
        }))),
        query: map(json!({
            "provider": "ASF",
            "dataset": "SENTINEL-1 BURSTS",
            "start": "2026-08-01T00:00:00Z",
            "end": "2026-08-13T00:00:00Z",
            "max_results": 10_000
        })),
        search_stats: frozen_candidate_count.map(|frozen_candidate_count| FrozenSearchStats {
            matched_count: Some(12_500),
            fetched_record_count: Some(10_000),
            mapped_record_count: Some(10_000),
            provider_returned_count: 10_000,
            frozen_candidate_count,
            truncated: true,
            source_attempts: vec![serde_json::from_value(json!({
                "provider_id": "asf.sentinel1",
                "native_collection_id": "SENTINEL-1",
                "outcome": "succeeded",
                "fallback_reason": null
            }))
            .unwrap()],
            source: "ASF_BATCH".to_owned(),
        }),
        candidates: (0..10_000).map(burst_candidate).collect(),
    };
    let batch_request = StoreSearchBatchRequest {
        activate_workbench: true,
        batch,
        workbench: SearchWorkbenchState {
            batch_id: BATCH_ID.to_owned(),
            checked_item_ids: vec!["BURST-00000".to_owned(), "BURST-09999".to_owned()],
            page_index: 9,
            page_size: 100,
            revision: 4,
        },
    };
    let task_request = StoreCreateTaskRequest {
        task_id: CURRENT_TASK_ID.to_owned(),
        batch_id: BATCH_ID.to_owned(),
        selected_item_ids: vec!["BURST-00000".to_owned()],
        checked_item_ids: vec!["BURST-00000".to_owned()],
        page_index: 0,
        page_size: 24,
        output_root: database.path().parent().unwrap().join("current-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-08-13T01:01:00Z".to_owned(),
    };
    store
        .save_search_batch_and_create_task_with_projection(
            &batch_request,
            &task_request,
            Some(&TaskOperationDescriptor {
                operation_id: "fixture-operation".to_owned(),
                kind: "dem".to_owned(),
                phase: "plan".to_owned(),
                region_id: "region-fixture".to_owned(),
                approved_worker_operation: "dem_inspect".to_owned(),
                payload: map(json!({
                    "candidates": [{"item_id": "MUST-NOT-CROSS-CORE-RESTORE"}],
                    "selected_item_ids": ["MUST-NOT-CROSS-CORE-RESTORE"]
                })),
            }),
            None,
        )
        .unwrap();
}

/// Sanitized structural reduction of the reviewed real compact-v2 record.
/// It intentionally preserves only the authoritative compatibility paths:
/// `query.dataset`, `query.processing_levels`, and
/// `selected_products[*].metadata.{dataset,processing_level,product_family,product_type,provider}`.
fn legacy_compact_snapshot(task_id: &str, processing_levels: Value) -> Value {
    json!({
        "persistence_version": 2,
        "schema_version": 1,
        "task_id": task_id,
        "batch_id": BATCH_ID,
        "provider_id": "asf.sentinel1",
        "created_at": CREATED_AT,
        "aoi_name": "Sanitized Legacy AOI",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 20.0], [111.0, 20.0], [111.0, 21.0], [110.0, 21.0], [110.0, 20.0]]]
        },
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": processing_levels
        },
        "selected_products": [{
            "item_id": format!("{task_id}-item"),
            "provider_id": "asf.sentinel1",
            "display_name": "Sanitized Sentinel-1 SLC",
            "metadata": {
                "dataset": null,
                "processing_level": "SLC",
                "product_family": null,
                "product_type": "SLC",
                "provider": null
            },
            "payload": {}
        }],
        "initial_selected_item_ids": [format!("{task_id}-item")],
        "output_root": format!("fixture-output-{task_id}"),
        "layout": "sentinel1_slc",
        "operation": null,
        "orbit_association": null
    })
}

fn legacy_runtime(task_id: &str) -> Value {
    let item_id = format!("{task_id}-item");
    json!({
        "task_id": task_id,
        "status": "queued",
        "selected_item_ids": [item_id.clone()],
        "checked_item_ids": [item_id.clone()],
        "page_index": 0,
        "page_size": 24,
        "active_item_ids": [],
        "paused_item_ids": [],
        "queued_item_ids": [item_id],
        "completed_item_ids": [],
        "failed_item_ids": [],
        "skipped_item_ids": [],
        "cancelled_item_ids": [],
        "task_paused": false,
        "concurrency": 2,
        "revision": 0,
        "updated_at": CREATED_AT
    })
}

fn insert_legacy_task(connection: &Connection, task_id: &str, snapshot: &Value) {
    let runtime = legacy_runtime(task_id);
    insert_raw_task(connection, task_id, &snapshot.to_string(), &runtime);
}

fn insert_raw_task(connection: &Connection, task_id: &str, snapshot_json: &str, runtime: &Value) {
    insert_raw_task_with_provider(connection, task_id, "asf.sentinel1", snapshot_json, runtime);
}

fn insert_raw_task_with_provider(
    connection: &Connection,
    task_id: &str,
    row_provider_id: &str,
    snapshot_json: &str,
    runtime: &Value,
) {
    connection
        .execute(
            "INSERT INTO task_snapshots( \
                task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json \
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'sentinel1_slc', ?6)",
            params![
                task_id,
                BATCH_ID,
                row_provider_id,
                CREATED_AT,
                format!("fixture-output-{task_id}"),
                snapshot_json
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json) \
             VALUES (?1, 'queued', 0, ?2, ?3)",
            params![task_id, CREATED_AT, runtime.to_string()],
        )
        .unwrap();
}

fn raw_task_json(connection: &Connection, task_id: &str) -> (Vec<u8>, Vec<u8>) {
    connection
        .query_row(
            "SELECT CAST(s.snapshot_json AS BLOB), CAST(r.state_json AS BLOB) \
             FROM task_snapshots s JOIN task_runtime r ON r.task_id = s.task_id \
             WHERE s.task_id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

#[test]
fn core_restore_is_candidate_free_and_isolates_exact_and_ambiguous_legacy_tasks() {
    let database = TempDatabase::new();
    seed_current_store(&database);

    let exact = legacy_compact_snapshot(ADAPTED_TASK_ID, json!(["SLC"]));
    let ambiguous = legacy_compact_snapshot(UNAVAILABLE_TASK_ID, json!(["SLC", "GRD_HD"]));
    assert_eq!(exact.pointer("/query/dataset"), Some(&json!("SENTINEL-1")));
    assert_eq!(
        exact.pointer("/query/processing_levels"),
        Some(&json!(["SLC"]))
    );
    assert_eq!(
        exact.pointer("/selected_products/0/metadata/dataset"),
        Some(&Value::Null)
    );
    assert_eq!(
        exact.pointer("/selected_products/0/metadata/processing_level"),
        Some(&json!("SLC"))
    );
    assert_eq!(
        exact.pointer("/selected_products/0/metadata/product_family"),
        Some(&Value::Null)
    );
    assert_eq!(
        exact.pointer("/selected_products/0/metadata/product_type"),
        Some(&json!("SLC"))
    );
    assert_eq!(
        exact.pointer("/selected_products/0/metadata/provider"),
        Some(&Value::Null)
    );

    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    insert_legacy_task(&connection, ADAPTED_TASK_ID, &exact);
    insert_legacy_task(&connection, UNAVAILABLE_TASK_ID, &ambiguous);
    let adapted_before = raw_task_json(&connection, ADAPTED_TASK_ID);
    let unavailable_before = raw_task_json(&connection, UNAVAILABLE_TASK_ID);
    drop(connection);

    let mut store = TaskStore::open(database.path()).unwrap();
    let restored = store.core_restore(false).unwrap();
    let serialized = serde_json::to_value(&restored).unwrap();

    assert_eq!(restored.readiness.schema_version, STORE_SCHEMA_VERSION);
    assert_eq!(
        restored.workspace_context.as_ref().unwrap().workspace_id,
        "workspace-fixture"
    );
    assert_eq!(
        restored.region_aoi.as_ref().unwrap().region_id,
        "region-fixture"
    );
    assert_eq!(restored.region_aois.len(), 1);
    assert_eq!(restored.tasks.len(), 1);
    assert_eq!(restored.tasks[0].task_id, CURRENT_TASK_ID);
    assert!(restored.tasks[0].actionable);
    assert_eq!(restored.tasks[0].selected_count, 1);

    let workbench = restored.workbench.as_ref().unwrap();
    assert_eq!(workbench.batch_id, BATCH_ID);
    assert_eq!(workbench.candidate_count, 10_000);
    assert_eq!(workbench.checked_count, 2);
    assert_eq!(workbench.page_index, 9);
    assert_eq!(workbench.page_size, 100);
    assert_eq!(workbench.revision, 4);
    assert!(workbench.deferred);

    assert_eq!(restored.legacy_task_warnings.len(), 2);
    assert_eq!(
        restored.legacy_task_warnings[0].compatibility_status,
        LegacyTaskCompatibilityStatus::AdaptedReadOnly
    );
    assert_eq!(restored.legacy_task_warnings[0].task_id, ADAPTED_TASK_ID);
    assert_eq!(
        restored.legacy_task_warnings[1].compatibility_status,
        LegacyTaskCompatibilityStatus::Unavailable
    );
    assert_eq!(
        restored.legacy_task_warnings[1].task_id,
        UNAVAILABLE_TASK_ID
    );

    let serialized_text = serialized.to_string();
    assert!(!serialized_text.contains("\"candidates\""));
    assert!(!serialized_text.contains("BURST-09999"));
    assert!(!serialized_text.contains("selected_products"));
    assert!(!serialized_text.contains("selected_item_ids"));
    assert!(!serialized_text.contains("MUST-NOT-CROSS-CORE-RESTORE"));
    assert_eq!(
        restored.tasks[0].operation.as_ref().unwrap().operation_id,
        "fixture-operation"
    );

    let scheduler_ids: Vec<String> = store
        .load_scheduler_tasks()
        .unwrap()
        .into_iter()
        .map(|task| task.snapshot.task_id)
        .collect();
    assert_eq!(scheduler_ids, [CURRENT_TASK_ID]);
    for task_id in [ADAPTED_TASK_ID, UNAVAILABLE_TASK_ID] {
        assert_eq!(
            store
                .load_scheduler_control_runtime(task_id)
                .unwrap()
                .unwrap()
                .task_id,
            task_id
        );
        let action_error = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::PauseTask,
                    item_ids: vec![],
                },
                event_id: format!("{task_id}-pause"),
                occurred_at: "2026-08-13T02:00:00Z".to_owned(),
            })
            .unwrap_err();
        assert_eq!(action_error.code(), "V3_STORE_NOT_FOUND");
        let append_error = store
            .append_task_items(&StoreAppendTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision: 0,
                item_ids: vec!["BURST-00001".to_owned()],
                event_id: format!("{task_id}-append"),
                occurred_at: "2026-08-13T02:00:00Z".to_owned(),
            })
            .unwrap_err();
        assert_eq!(append_error.code(), "V3_STORE_NOT_FOUND");
        let view_error = store
            .update_task_view(&StoreUpdateTaskViewRequest {
                task_id: task_id.to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                updated_at: "2026-08-13T02:00:00Z".to_owned(),
            })
            .unwrap_err();
        assert_eq!(view_error.code(), "V3_STORE_NOT_FOUND");
        assert_eq!(
            store.load_download_snapshot(task_id).unwrap_err().code(),
            "V3_STORE_NOT_FOUND"
        );
        assert!(store.load_task(task_id).is_err());
        assert_eq!(
            store.list_task_events(task_id).unwrap_err().code(),
            "V3_STORE_NOT_FOUND"
        );
        let progress_error = store
            .save_download_progress(
                &DownloadItemProgress {
                    task_id: task_id.to_owned(),
                    item_id: format!("{task_id}-item"),
                    status: DownloadItemStatus::Active,
                    bytes_completed: 1,
                    total_bytes: Some(10),
                    bytes_per_second: 1.0,
                    attempt: 1,
                    updated_at: "2026-08-13T02:00:00Z".to_owned(),
                },
                0,
            )
            .unwrap_err();
        assert_eq!(progress_error.code(), "V3_STORE_NOT_FOUND");
    }
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    assert_eq!(raw_task_json(&connection, ADAPTED_TASK_ID), adapted_before);
    assert_eq!(
        raw_task_json(&connection, UNAVAILABLE_TASK_ID),
        unavailable_before
    );
}

#[test]
fn candidate_free_fixture_metrics_are_reproducible() {
    let database = TempDatabase::new();
    seed_current_store(&database);
    let store = TaskStore::open(database.path()).unwrap();
    let connection = Connection::open(database.path()).unwrap();
    let sqlite_version: String = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .unwrap();
    let json1_available: i64 = connection
        .query_row("SELECT json_valid('{\"ok\":true}')", [], |row| row.get(0))
        .unwrap();

    let core_started = std::time::Instant::now();
    let core = store.core_restore(false).unwrap();
    let core_elapsed_ms = core_started.elapsed().as_secs_f64() * 1_000.0;
    let core_payload_bytes = serde_json::to_vec(&core).unwrap().len();
    assert_eq!(core.workbench.as_ref().unwrap().candidate_count, 10_000);
    assert!(!serde_json::to_string(&core)
        .unwrap()
        .contains("\"candidates\""));

    let mut page_metrics = Vec::new();
    for page_index in [0_u64, 50, 99] {
        let started = std::time::Instant::now();
        let page = store
            .load_workbench_page(&StoreWorkbenchPageRequest {
                batch_id: BATCH_ID.to_owned(),
                expected_revision: 4,
                page_index,
                page_size: 100,
            })
            .unwrap();
        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let payload_bytes = serde_json::to_vec(&page).unwrap().len();
        assert_eq!(page.items.len(), 100);
        page_metrics.push((page_index, elapsed_ms, payload_bytes));
    }

    eprintln!(
        "UA0_FIXTURE_METRICS sqlite={sqlite_version} json1={json1_available} candidates=10000 core_candidates_transmitted=0 core_elapsed_ms={core_elapsed_ms:.3} core_payload_bytes={core_payload_bytes} page_size=100 page_metrics={page_metrics:?}"
    );
}

#[test]
fn core_restore_carries_fixed_search_stats_without_candidates_and_old_batches_remain_honest() {
    let database = TempDatabase::new();
    seed_current_store(&database);
    let store = TaskStore::open(database.path()).unwrap();
    let restored = store.core_restore(false).unwrap();
    let serialized = serde_json::to_value(&restored).unwrap();

    assert_eq!(
        serialized.pointer("/workbench/search_stats/matched_count"),
        Some(&json!(12_500))
    );
    assert_eq!(
        serialized.pointer("/workbench/search_stats/provider_returned_count"),
        Some(&json!(10_000))
    );
    assert_eq!(
        serialized.pointer("/workbench/search_stats/frozen_candidate_count"),
        Some(&json!(10_000))
    );
    assert_eq!(
        serialized.pointer("/workbench/search_stats/source"),
        Some(&json!("ASF_BATCH"))
    );
    assert_eq!(
        serialized.pointer("/workbench/search_stats/source_attempts/0/provider_id"),
        Some(&json!("asf.sentinel1"))
    );
    assert_eq!(
        serialized.pointer("/workbench/search_stats/source_attempts/0/outcome"),
        Some(&json!("succeeded"))
    );
    assert_eq!(
        serialized.pointer("/workbench/query/target_count"),
        Some(&json!(10_000))
    );
    assert_eq!(
        serialized.pointer("/workbench/current_page_item_count"),
        Some(&json!(100))
    );
    assert!(!serialized.to_string().contains("BURST-09999"));
    drop(store);

    let legacy_database = TempDatabase::new();
    seed_current_store_with_search_stats(&legacy_database, None);
    let legacy_store = TaskStore::open(legacy_database.path()).unwrap();
    let legacy = serde_json::to_value(legacy_store.core_restore(false).unwrap()).unwrap();
    assert_eq!(
        legacy.pointer("/workbench/search_stats"),
        Some(&Value::Null)
    );

    let mismatched_database = TempDatabase::new();
    seed_current_store_with_search_stats(&mismatched_database, Some(9_999));
    let mismatched_store = TaskStore::open(mismatched_database.path()).unwrap();
    assert_eq!(
        mismatched_store.core_restore(false).unwrap_err().code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
}

#[test]
fn core_restore_reports_typed_logical_dataset_as_the_workbench_dataset() {
    const NISAR_DATASET: &str = "sar.nisar.l.rslc.provisional.v1";
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    let mut candidate = burst_candidate(0);
    candidate.item_id = "NISAR-00000".to_owned();
    candidate.provider_id = "asf.nasa.earthdata".to_owned();
    candidate.metadata.insert(
        "logical_dataset".to_owned(),
        Value::String(NISAR_DATASET.to_owned()),
    );
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "typed-nisar-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: CREATED_AT.to_owned(),
                aoi_name: "Typed NISAR AOI".to_owned(),
                aoi_geojson: None,
                query: map(json!({
                    "logical_dataset": NISAR_DATASET,
                    "provider_id": "asf.nasa.earthdata",
                    "native_collection_id": "C2853145197-ASF",
                    "aoi_geojson": null,
                    "cursor": null,
                    "result_limit": 200,
                    "filters": {}
                })),
                search_stats: None,
                candidates: vec![candidate],
            },
            workbench: SearchWorkbenchState {
                batch_id: "typed-nisar-batch".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();

    let restored = store.core_restore(false).unwrap();
    assert_eq!(
        restored.workbench.unwrap().query.dataset.as_deref(),
        Some(NISAR_DATASET)
    );
}

#[test]
fn malformed_or_mismatched_envelopes_become_unavailable_without_blocking_core_restore() {
    let database = TempDatabase::new();
    seed_current_store(&database);
    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();

    let malformed_task_id = "fixture-malformed-json";
    insert_raw_task(
        &connection,
        malformed_task_id,
        "{not-json",
        &legacy_runtime(malformed_task_id),
    );
    let mismatched_task_id = "fixture-envelope-mismatch";
    insert_raw_task_with_provider(
        &connection,
        mismatched_task_id,
        "wrong.provider",
        &legacy_compact_snapshot(mismatched_task_id, json!(["SLC"])).to_string(),
        &legacy_runtime(mismatched_task_id),
    );
    let unsupported_task_id = "fixture-unsupported-version";
    let mut unsupported = legacy_compact_snapshot(unsupported_task_id, json!(["SLC"]));
    unsupported["persistence_version"] = json!(4);
    insert_legacy_task(&connection, unsupported_task_id, &unsupported);
    let runtime_mismatch_task_id = "fixture-runtime-mismatch";
    insert_raw_task(
        &connection,
        runtime_mismatch_task_id,
        &legacy_compact_snapshot(runtime_mismatch_task_id, json!(["SLC"])).to_string(),
        &legacy_runtime("different-runtime-task"),
    );
    let empty_task_id = "fixture-empty-products";
    let mut empty = legacy_compact_snapshot(empty_task_id, json!(["SLC"]));
    empty["selected_products"] = json!([]);
    empty["initial_selected_item_ids"] = json!([]);
    let mut empty_runtime = legacy_runtime(empty_task_id);
    for field in ["selected_item_ids", "checked_item_ids", "queued_item_ids"] {
        empty_runtime[field] = json!([]);
    }
    insert_raw_task(
        &connection,
        empty_task_id,
        &empty.to_string(),
        &empty_runtime,
    );
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    let restored = store.core_restore(false).unwrap();
    assert_eq!(restored.tasks.len(), 1);
    assert_eq!(restored.legacy_task_warnings.len(), 5);
    assert!(restored.legacy_task_warnings.iter().all(|warning| {
        warning.compatibility_status == LegacyTaskCompatibilityStatus::Unavailable
    }));
    let reason = |task_id: &str| {
        restored
            .legacy_task_warnings
            .iter()
            .find(|warning| warning.task_id == task_id)
            .unwrap()
            .reason_code
            .clone()
    };
    assert_eq!(
        reason(malformed_task_id),
        LegacyTaskRestoreReasonCode::MalformedSnapshot
    );
    assert_eq!(
        reason(mismatched_task_id),
        LegacyTaskRestoreReasonCode::TaskEnvelopeMismatch
    );
    assert_eq!(
        reason(unsupported_task_id),
        LegacyTaskRestoreReasonCode::UnsupportedPersistenceVersion
    );
    assert_eq!(
        reason(runtime_mismatch_task_id),
        LegacyTaskRestoreReasonCode::RuntimeEnvelopeMismatch
    );
    assert_eq!(
        reason(empty_task_id),
        LegacyTaskRestoreReasonCode::AmbiguousLegacyEvidence
    );
    assert_eq!(
        store
            .load_scheduler_tasks()
            .unwrap()
            .into_iter()
            .map(|task| task.snapshot.task_id)
            .collect::<Vec<_>>(),
        [CURRENT_TASK_ID]
    );
    for task_id in [
        malformed_task_id,
        mismatched_task_id,
        unsupported_task_id,
        runtime_mismatch_task_id,
        empty_task_id,
    ] {
        assert!(store.load_scheduler_task(task_id).unwrap().is_none());
    }
}

#[test]
fn already_archived_degraded_task_is_hidden_from_every_history_path() {
    let database = TempDatabase::new();
    seed_current_store(&database);
    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    insert_legacy_task(
        &connection,
        ADAPTED_TASK_ID,
        &legacy_compact_snapshot(ADAPTED_TASK_ID, json!(["SLC"])),
    );
    connection
        .execute(
            "INSERT INTO task_history(task_id, archived_at, layout, record_json) \
             VALUES (?1, ?2, 'sentinel1_slc', '{}')",
            params![ADAPTED_TASK_ID, "2026-08-13T03:00:00Z"],
        )
        .unwrap();
    drop(connection);

    let mut store = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: ADAPTED_TASK_ID.to_owned(),
                expected_revision: 0,
                archived_at: "2026-08-13T03:00:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "V3_STORE_NOT_FOUND"
    );
    assert!(store.list_history().unwrap().is_empty());
    assert_eq!(
        store
            .delete_history(&StoreDeleteHistoryRequest {
                task_id: ADAPTED_TASK_ID.to_owned(),
                deleted_at: "2026-08-13T03:01:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "V3_STORE_NOT_FOUND"
    );
    let connection = Connection::open(database.path()).unwrap();
    let history_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM task_history WHERE task_id = ?1",
            [ADAPTED_TASK_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history_count, 1);
}
