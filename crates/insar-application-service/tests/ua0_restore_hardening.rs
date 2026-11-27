use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, MAX_PAGE_RENDER_JSON_BYTES,
};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreUpdateWorkbenchRequest, WorkspaceContextState,
    WorkspaceProjectState, WorkspaceRegionState,
};
use insar_store::TaskStore;
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

const BATCH_ID: &str = "ua0-service-batch-10000";
const CURRENT_TASK_ID: &str = "ua0-service-current-task";
const ADAPTED_TASK_ID: &str = "ua0-service-legacy-adapted";
const UNAVAILABLE_TASK_ID: &str = "ua0-service-legacy-unavailable";
const CREATED_AT: &str = "2026-08-13T06:00:00Z";

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-appsvc-ua0-{}-{nonce}-{seq}.sqlite3",
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

fn candidate(index: usize) -> SnapshotItem {
    SnapshotItem {
        item_id: format!("SVC-ITEM-{index:05}"),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("Sanitized service item {index:05}"),
        acquisition_datetime: Some("2026-08-13T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({"dataset": "SENTINEL-1 BURSTS"})),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed(database: &TempDatabase) {
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
            revision: 3,
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

    let batch_request = StoreSearchBatchRequest {
        activate_workbench: true,
        batch: FrozenSearchBatch {
            schema_version: 1,
            batch_id: BATCH_ID.to_owned(),
            provider_id: "asf.sentinel1".to_owned(),
            created_at: CREATED_AT.to_owned(),
            aoi_name: "Fixture AOI".to_owned(),
            aoi_geojson: Some(map(json!({
                "type": "Polygon",
                "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
            }))),
            query: map(json!({"dataset": "SENTINEL-1 BURSTS"})),
            search_stats: None,
            candidates: (0..10_000).map(candidate).collect(),
        },
        workbench: SearchWorkbenchState {
            batch_id: BATCH_ID.to_owned(),
            checked_item_ids: vec!["SVC-ITEM-00000".to_owned(), "SVC-ITEM-09999".to_owned()],
            page_index: 0,
            page_size: 100,
            revision: 2,
        },
    };
    let task_request = StoreCreateTaskRequest {
        task_id: CURRENT_TASK_ID.to_owned(),
        batch_id: BATCH_ID.to_owned(),
        selected_item_ids: vec!["SVC-ITEM-00000".to_owned()],
        checked_item_ids: vec!["SVC-ITEM-00000".to_owned()],
        page_index: 0,
        page_size: 24,
        output_root: database.path().parent().unwrap().join("current-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: CREATED_AT.to_owned(),
    };
    store
        .save_search_batch_and_create_task(&batch_request, &task_request)
        .unwrap();

    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    insert_legacy_task(&connection, ADAPTED_TASK_ID, json!(["SLC"]));
    insert_legacy_task(&connection, UNAVAILABLE_TASK_ID, json!(["SLC", "GRD_HD"]));
    drop(connection);
}

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

fn insert_legacy_task(connection: &Connection, task_id: &str, processing_levels: Value) {
    let snapshot = legacy_compact_snapshot(task_id, processing_levels);
    let runtime = json!({
        "task_id": task_id,
        "status": "queued",
        "selected_item_ids": [format!("{task_id}-item")],
        "checked_item_ids": [format!("{task_id}-item")],
        "page_index": 0,
        "page_size": 24,
        "active_item_ids": [],
        "paused_item_ids": [],
        "queued_item_ids": [format!("{task_id}-item")],
        "completed_item_ids": [],
        "failed_item_ids": [],
        "skipped_item_ids": [],
        "cancelled_item_ids": [],
        "task_paused": false,
        "concurrency": 2,
        "revision": 0,
        "updated_at": CREATED_AT
    });
    connection
        .execute(
            "INSERT INTO task_snapshots( \
                task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json \
             ) VALUES (?1, ?2, 'asf.sentinel1', ?3, ?4, 'sentinel1_slc', ?5)",
            params![
                task_id,
                BATCH_ID,
                CREATED_AT,
                format!("fixture-output-{task_id}"),
                snapshot.to_string()
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

#[test]
fn core_restore_and_workbench_page_are_bounded_and_legacy_isolated() {
    let database = TempDatabase::new();
    seed(&database);
    let service = ApplicationService::new(ApplicationServiceConfig::store(database.path()));

    let snapshot = service.core_restore(false).unwrap();
    assert_eq!(
        snapshot
            .core
            .workspace_context
            .as_ref()
            .unwrap()
            .workspace_id,
        "workspace-fixture"
    );
    assert_eq!(
        snapshot.core.region_aoi.as_ref().unwrap().region_id,
        "region-fixture"
    );
    assert_eq!(snapshot.core.tasks.len(), 1);
    assert_eq!(snapshot.core.tasks[0].task_id, CURRENT_TASK_ID);
    assert!(snapshot.core.tasks[0].actionable);
    assert_eq!(snapshot.core.legacy_task_warnings.len(), 2);
    assert_eq!(
        snapshot
            .core
            .legacy_task_warnings
            .iter()
            .map(|warning| warning.task_id.as_str())
            .collect::<Vec<_>>(),
        [ADAPTED_TASK_ID, UNAVAILABLE_TASK_ID]
    );
    let workbench = snapshot.core.workbench.as_ref().unwrap();
    assert_eq!(workbench.candidate_count, 10_000);
    assert_eq!(workbench.checked_count, 2);

    let wire = serde_json::to_value(&snapshot).unwrap().to_string();
    assert!(!wire.contains("\"candidates\""));
    assert!(!wire.contains("SVC-ITEM-09999"));
    assert!(!wire.contains("selected_products"));

    let page = service
        .workbench_page(&insar_store::StoreWorkbenchPageRequest {
            batch_id: BATCH_ID.to_owned(),
            expected_revision: 2,
            page_index: 99,
            page_size: 100,
        })
        .unwrap();
    assert_eq!(page.page.items.len(), 100);
    assert_eq!(page.page.items[0].ordinal, 9_900);
    assert_eq!(page.page.items[99].ordinal, 9_999);
    assert!(page.page.items[99].checked);
    assert_eq!(page.page.total_count, 10_000);
    let render = page
        .render
        .as_ref()
        .expect("native page render authority must share the AOI snapshot");
    assert_eq!(render.region_id, "region-fixture");
    assert_eq!(render.revision, 0);
    assert_eq!(render.items.len(), page.page.items.len());
    assert!(render
        .items
        .iter()
        .all(|item| item.render_geometry.is_none() && item.render_bbox.is_none()));
    assert!(serde_json::to_vec(render).unwrap().len() <= MAX_PAGE_RENDER_JSON_BYTES);

    // The legacy full-restore route remains only as a compatibility reader.
    let compat = service.snapshot(false).unwrap();
    assert_eq!(compat.restore.tasks.len(), 3);
}

#[test]
fn core_restore_exposes_native_aoi_display_authority_without_store_mutation() {
    let database = TempDatabase::new();
    seed(&database);
    let service = ApplicationService::new(ApplicationServiceConfig::store(database.path()));

    let snapshot = service.core_restore(false).unwrap();
    let display = snapshot
        .aoi_display
        .as_ref()
        .expect("native AOI display authority must be derived for a valid AOI");
    assert_eq!(display.region_id, "region-fixture");
    assert_eq!(display.revision, 0);
    assert_eq!(display.envelope.display_west, 100.0);
    assert_eq!(display.envelope.display_east, 101.0);
    assert_eq!(display.envelope.request_windows.len(), 1);
    let persisted_region_aoi = snapshot.core.region_aoi.clone();

    let wire = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(wire["aoi_display"]["region_id"], "region-fixture");
    assert_eq!(wire["aoi_display"]["envelope"]["display_west"], 100.0);
    assert_eq!(
        wire["aoi_display"]["render_geometry"]["type"],
        "MultiPolygon"
    );
    assert_eq!(wire["aoi_display"]["render_bbox"]["west"], 100.0);
    assert_eq!(wire["core"]["region_aoi"]["geometry"]["type"], "Polygon");
    assert_eq!(
        service.core_restore(false).unwrap().core.region_aoi,
        persisted_region_aoi,
    );
}

#[test]
fn ordinary_workbench_mutation_returns_bounded_core_snapshot() {
    let database = TempDatabase::new();
    seed(&database);
    let service = ApplicationService::new(ApplicationServiceConfig::offline_smoke(database.path()));

    let response = service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: BATCH_ID.to_owned(),
                checked_item_ids: vec!["SVC-ITEM-00000".to_owned(), "SVC-ITEM-09999".to_owned()],
                page_index: 4,
                page_size: 50,
                revision: 3,
            },
            expected_revision: 2,
            updated_at: "2026-08-13T06:01:00Z".to_owned(),
        })
        .unwrap();
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["aoi_display", "core", "readiness"]
    );
    assert_eq!(value["core"]["workbench"]["page_index"], 4);
    assert_eq!(value["core"]["workbench"]["page_size"], 50);
    assert!(value["core"].get("tasks").is_some());
    assert!(value.to_string().len() < 100_000);
    assert!(!value.to_string().contains("SVC-ITEM-09999"));
}

#[test]
fn verification_only_profile_reads_without_mutation_capability() {
    let database = TempDatabase::new();
    seed(&database);

    // Quiesce the seeded Store so the verification read has no WAL/SHM
    // sidecars, matching a stable production Store that is not being written.
    {
        let connection = Connection::open(database.path()).unwrap();
        connection
            .pragma_update(None, "journal_mode", "DELETE")
            .unwrap();
    }

    let service =
        ApplicationService::new(ApplicationServiceConfig::verification_only(database.path()));
    service.startup().unwrap();
    let core = service.core_restore(false).unwrap();
    assert_eq!(core.core.tasks.len(), 1);
    assert_eq!(
        core.core.workbench.as_ref().unwrap().candidate_count,
        10_000
    );
    let page = service
        .workbench_page(&insar_store::StoreWorkbenchPageRequest {
            batch_id: BATCH_ID.to_owned(),
            expected_revision: 2,
            page_index: 0,
            page_size: 100,
        })
        .unwrap();
    assert_eq!(page.page.items.len(), 100);

    // The verification Store lease rejects every write at the SQLite level.
    let mut store = TaskStore::open_verification_read_only(database.path()).unwrap();
    assert!(store
        .apply_workbench_selection(&insar_core::WorkbenchSelectionCommand {
            batch_id: BATCH_ID.to_owned(),
            expected_revision: 2,
            kind: insar_core::WorkbenchSelectionCommandKind::ClearAll,
            item_ids: vec![],
            page_index: 0,
            page_size: 100,
            updated_at: CREATED_AT.to_owned(),
        })
        .is_err());
    assert!(
        store.health().unwrap().journal_mode == "delete",
        "verification read must not change journal mode"
    );

    // A WAL sidecar makes the verification read fail closed.
    let sidecar = PathBuf::from(format!("{}-wal", database.path().display()));
    fs::write(&sidecar, b"fake wal").unwrap();
    let error = TaskStore::open_verification_read_only(database.path()).unwrap_err();
    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    fs::remove_file(sidecar).unwrap();

    // Business mutations are structurally unavailable in verification mode.
    let error = service
        .create_dem_task(&insar_application_service::DemTaskRequest {
            plan: insar_application_service::DemTaskPlan {
                request_id: "verification-mutation".to_owned(),
                task_id: "verification-task".to_owned(),
                region_id: "region-fixture".to_owned(),
                resource_id: "dem.fixture".to_owned(),
                capability_id: "dem.inspect".to_owned(),
                dataset: "COP-DEM-GLO-30".to_owned(),
                provider: "copernicus".to_owned(),
                operation: insar_application_service::DemTaskOperation::Inspect,
                input_path: PathBuf::from("fixture-input"),
                output_root: database
                    .path()
                    .parent()
                    .unwrap()
                    .join("verification-output"),
                source_vertical_datum: None,
                target_vertical_datum: None,
                output_mode: None,
                created_at: CREATED_AT.to_owned(),
            },
            concurrency: 2,
        })
        .unwrap_err();
    assert!(
        error.code() == "FORMAL_TASK_ROUTE_UNAVAILABLE"
            || error.code() == "V3_APPLICATION_BUSINESS_RUNTIME_UNAVAILABLE",
        "unexpected mutation error code: {}",
        error.code()
    );
}
