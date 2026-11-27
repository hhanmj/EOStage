use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreDeleteHistoryRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskOperationDescriptor, TaskStateCommandKind,
};
use insar_store::{
    StoreDemConversionSource, StoreDemConversionSourceRequest, TaskStore, STORE_SCHEMA_VERSION,
};
use rusqlite::Connection;
use serde_json::{Map, Value};

const DATASET: &str = "COP30";
const PROVIDER: &str = "COPERNICUS_AWS";
const RESOURCE_ID: &str = "dem.cop30.v1";
const CAPABILITY_ID: &str = "capability.dem.cop30.aws-download.v1";

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
            "insar-dem-conversion-source-{}-{nonce}-{seq}.sqlite3",
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

fn item(task_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: format!("ITEM-{task_id}"),
        provider_id: "compat.dem".to_owned(),
        display_name: format!("DEM {task_id}"),
        acquisition_datetime: Some("2026-08-17T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn create_task(
    store: &mut TaskStore,
    database: &TempDatabase,
    task_id: &str,
    region_id: &str,
    phase: &str,
    worker: &str,
    created_at: &str,
) {
    create_task_with_kind(
        store, database, task_id, "dem", region_id, phase, worker, created_at,
    );
}

// Keep the fixture's authority inputs explicit at each regression call site.
#[allow(clippy::too_many_arguments)]
fn create_task_with_region_alias(
    store: &mut TaskStore,
    database: &TempDatabase,
    task_id: &str,
    region_id: &str,
    phase: &str,
    worker: &str,
    created_at: &str,
    region_id_alias: Option<Value>,
) {
    let output_root = database.path().parent().expect("parent").join(task_id);
    let input_path = output_root.join("source.tif");
    create_task_with_authority_and_region_alias(
        store,
        database,
        task_id,
        "dem",
        region_id,
        phase,
        worker,
        created_at,
        DATASET,
        PROVIDER,
        RESOURCE_ID,
        CAPABILITY_ID,
        &input_path,
        &output_root,
        region_id_alias,
    );
}

#[allow(clippy::too_many_arguments)]
fn create_task_with_kind(
    store: &mut TaskStore,
    database: &TempDatabase,
    task_id: &str,
    kind: &str,
    region_id: &str,
    phase: &str,
    worker: &str,
    created_at: &str,
) {
    let output_root = database.path().parent().expect("parent").join(task_id);
    let input_path = output_root.join("source.tif");
    create_task_with_authority(
        store,
        database,
        task_id,
        kind,
        region_id,
        phase,
        worker,
        created_at,
        DATASET,
        PROVIDER,
        RESOURCE_ID,
        CAPABILITY_ID,
        &input_path,
        &output_root,
    );
}

#[allow(clippy::too_many_arguments)]
fn create_task_with_authority(
    store: &mut TaskStore,
    database: &TempDatabase,
    task_id: &str,
    kind: &str,
    region_id: &str,
    phase: &str,
    worker: &str,
    created_at: &str,
    dataset: &str,
    provider: &str,
    resource_id: &str,
    capability_id: &str,
    input_path: &Path,
    output_root: &Path,
) {
    create_task_with_authority_and_region_alias(
        store,
        database,
        task_id,
        kind,
        region_id,
        phase,
        worker,
        created_at,
        dataset,
        provider,
        resource_id,
        capability_id,
        input_path,
        output_root,
        Some(Value::String(region_id.to_owned())),
    );
}

#[allow(clippy::too_many_arguments)]
fn create_task_with_authority_and_region_alias(
    store: &mut TaskStore,
    _database: &TempDatabase,
    task_id: &str,
    kind: &str,
    region_id: &str,
    phase: &str,
    worker: &str,
    created_at: &str,
    dataset: &str,
    provider: &str,
    resource_id: &str,
    capability_id: &str,
    input_path: &Path,
    output_root: &Path,
    region_id_alias: Option<Value>,
) {
    let batch_id = format!("batch-{task_id}");
    let item = item(task_id);
    let item_id = item.item_id.clone();
    let mut payload = Map::<String, Value>::from_iter([
        (
            "input_path".to_owned(),
            Value::String(input_path.to_string_lossy().to_string()),
        ),
        (
            "path".to_owned(),
            Value::String(input_path.to_string_lossy().to_string()),
        ),
        (
            "output_root".to_owned(),
            Value::String(output_root.to_string_lossy().to_string()),
        ),
        (
            "output_dir".to_owned(),
            Value::String(output_root.to_string_lossy().to_string()),
        ),
        ("dataset".to_owned(), Value::String(dataset.to_owned())),
        ("provider".to_owned(), Value::String(provider.to_owned())),
        (
            "resource_id".to_owned(),
            Value::String(resource_id.to_owned()),
        ),
        (
            "capability_id".to_owned(),
            Value::String(capability_id.to_owned()),
        ),
    ]);
    if let Some(region_id_alias) = region_id_alias {
        payload.insert("region_id".to_owned(), region_id_alias);
    }
    store
        .save_search_batch_and_create_task_with_projection(
            &StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.clone(),
                    provider_id: "compat.dem".to_owned(),
                    created_at: created_at.to_owned(),
                    aoi_name: "DEM AOI".to_owned(),
                    aoi_geojson: Some(
                        serde_json::json!({
                            "type": "Polygon",
                            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
                        })
                        .as_object()
                        .expect("AOI")
                        .clone(),
                    ),
                    query: Map::new(),
                    search_stats: None,
                    candidates: vec![item],
                },
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.clone(),
                    checked_item_ids: vec![item_id.clone()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            },
            &StoreCreateTaskRequest {
                task_id: task_id.to_owned(),
                batch_id,
                selected_item_ids: vec![item_id.clone()],
                checked_item_ids: vec![item_id],
                page_index: 0,
                page_size: 24,
                output_root: output_root.to_owned(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: created_at.to_owned(),
            },
            Some(&TaskOperationDescriptor {
                operation_id: format!("{task_id}:prepare_sarscape"),
                kind: kind.to_owned(),
                phase: phase.to_owned(),
                region_id: region_id.to_owned(),
                approved_worker_operation: worker.to_owned(),
                payload,
            }),
            None,
        )
        .expect("create projected DEM task");
}

fn complete(store: &mut TaskStore, task_id: &str, occurred_at: &str) {
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec![format!("ITEM-{task_id}")],
            },
            event_id: format!("complete-{task_id}"),
            occurred_at: occurred_at.to_owned(),
        })
        .expect("complete DEM task");
}

fn archive(store: &mut TaskStore, task_id: &str, archived_at: &str) {
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 1,
            archived_at: archived_at.to_owned(),
        })
        .expect("archive DEM task");
}

fn source_request(region_id: &str) -> StoreDemConversionSourceRequest {
    StoreDemConversionSourceRequest {
        region_id: region_id.to_owned(),
        dataset: DATASET.to_owned(),
        provider: PROVIDER.to_owned(),
        resource_id: RESOURCE_ID.to_owned(),
        capability_id: CAPABILITY_ID.to_owned(),
    }
}

fn resolve(store: &TaskStore, region_id: &str) -> Option<StoreDemConversionSource> {
    store
        .resolve_dem_conversion_source(&source_request(region_id))
        .expect("resolve DEM source")
}

#[test]
fn archived_completed_dem_source_survives_active_only_restore_and_reopen() {
    let database = TempDatabase::new();
    let custom_output_root = database.path().parent().unwrap().join("custom-dem-output");
    let custom_input_path = database
        .path()
        .parent()
        .unwrap()
        .join("custom-input/source-cop30.tif");
    {
        let mut store = TaskStore::open(database.path()).expect("open store");
        create_task_with_authority(
            &mut store,
            &database,
            "archived-source",
            "dem",
            "region-dem",
            "execute",
            "dem_prepare_sarscape",
            "2026-08-17T00:00:00Z",
            DATASET,
            PROVIDER,
            RESOURCE_ID,
            CAPABILITY_ID,
            &custom_input_path,
            &custom_output_root,
        );
        complete(&mut store, "archived-source", "2026-08-17T00:01:00Z");
        archive(&mut store, "archived-source", "2026-08-17T00:02:00Z");
        assert!(store
            .core_restore(false)
            .expect("active restore")
            .tasks
            .is_empty());
        let source = resolve(&store, "region-dem").expect("archived source");
        assert_eq!(source.task_id, "archived-source");
        assert_eq!(source.revision, 1);
        assert_eq!(source.dataset, DATASET);
        assert_eq!(source.provider, PROVIDER);
        assert_eq!(source.resource_id, RESOURCE_ID);
        assert_eq!(source.capability_id, CAPABILITY_ID);
    }

    // A synthesized v9 fixture must not retain tables first introduced by
    // later schema versions; otherwise v27 migration correctly rejects them.
    let connection = Connection::open(database.path()).expect("open v9 fixture");
    connection
        .execute_batch(
            "DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             DROP TABLE task_history_dem_source_authority;
             DROP TABLE dem_source_identity;
             DROP TABLE standalone_aoi_state;
             DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2;
             PRAGMA user_version = 9;",
        )
        .expect("downgrade to v9 authority fixture");
    drop(connection);

    let reopened = TaskStore::open(database.path()).expect("reopen store");
    assert_eq!(
        reopened.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert!(reopened
        .core_restore(false)
        .expect("active restore after restart")
        .tasks
        .is_empty());
    let source = resolve(&reopened, "region-dem").expect("reopened archived source");
    assert_eq!(source.task_id, "archived-source");
    assert_eq!(source.input_path, custom_input_path);
    assert_eq!(source.output_root, custom_output_root);
    drop(reopened);
    let reopened = TaskStore::open_existing(database.path()).expect("idempotent v10 reopen");
    assert_eq!(
        resolve(&reopened, "region-dem").unwrap().task_id,
        "archived-source"
    );
}

#[test]
fn core_restore_projects_dem_dataset_and_scientific_source_from_operation_payload() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    create_task(
        &mut store,
        &database,
        "core-dem-source",
        "region-core-dem",
        "download",
        "dem_download",
        "2026-08-17T00:00:00Z",
    );

    let restored = store.core_restore(false).expect("core restore");
    let task = restored
        .tasks
        .iter()
        .find(|task| task.task_id == "core-dem-source")
        .expect("DEM task summary");
    assert_eq!(task.provider_id, "compat.dem");
    assert_eq!(task.dataset_id.as_deref(), Some(DATASET));
    assert_eq!(
        task.operation
            .as_ref()
            .and_then(|operation| operation.source_provider.as_deref()),
        Some(PROVIDER)
    );
}

#[test]
fn resolver_accepts_native_aoi_dem_download_for_live_task() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    create_task_with_region_alias(
        &mut store,
        &database,
        "native-live-source",
        "region-native-live",
        "download",
        "dem_download",
        "2026-08-17T00:00:00Z",
        None,
    );
    complete(&mut store, "native-live-source", "2026-08-17T00:01:00Z");

    let source = resolve(&store, "region-native-live").expect("native live source");
    assert_eq!(source.task_id, "native-live-source");
    assert_eq!(source.revision, 1);
    assert_eq!(source.region_id, "region-native-live");
    assert_eq!(source.dataset, DATASET);
    assert_eq!(source.provider, PROVIDER);
    assert_eq!(source.resource_id, RESOURCE_ID);
    assert_eq!(source.capability_id, CAPABILITY_ID);
}

#[test]
fn resolver_accepts_native_aoi_dem_download_for_archived_task() {
    let database = TempDatabase::new();
    let output_root = database
        .path()
        .parent()
        .unwrap()
        .join("native-archive-source");
    let input_path = output_root.join("source.tif");
    {
        let mut store = TaskStore::open(database.path()).expect("open store");
        create_task_with_region_alias(
            &mut store,
            &database,
            "native-archive-source",
            "region-native-archive",
            "download",
            "dem_download",
            "2026-08-17T00:00:00Z",
            None,
        );
        complete(&mut store, "native-archive-source", "2026-08-17T00:01:00Z");
        archive(&mut store, "native-archive-source", "2026-08-17T00:02:00Z");

        assert!(store
            .core_restore(false)
            .expect("active restore")
            .tasks
            .is_empty());
        let source = resolve(&store, "region-native-archive").expect("native archive source");
        assert_eq!(source.task_id, "native-archive-source");
        assert_eq!(source.revision, 1);
        assert_eq!(source.input_path, input_path);
        assert_eq!(source.output_root, output_root);
    }

    let reopened = TaskStore::open_existing(database.path()).expect("reopen store");
    let source = resolve(&reopened, "region-native-archive").expect("reopened native source");
    assert_eq!(source.task_id, "native-archive-source");
    assert_eq!(source.input_path, input_path);
    assert_eq!(source.output_root, output_root);
}

#[test]
fn resolver_accepts_native_aoi_dem_download_with_matching_region_alias() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    create_task_with_region_alias(
        &mut store,
        &database,
        "native-matching-alias",
        "region-native-alias",
        "download",
        "dem_download",
        "2026-08-17T00:00:00Z",
        Some(Value::String("region-native-alias".to_owned())),
    );
    complete(&mut store, "native-matching-alias", "2026-08-17T00:01:00Z");

    let source = resolve(&store, "region-native-alias").expect("native matching source");
    assert_eq!(source.task_id, "native-matching-alias");
    assert_eq!(source.region_id, "region-native-alias");
}

#[test]
fn resolver_rejects_native_aoi_dem_download_with_invalid_region_alias() {
    for (task_id, region_id_alias) in [
        ("native-null-alias", Value::Null),
        (
            "native-wrong-alias",
            Value::String("region-other".to_owned()),
        ),
    ] {
        let database = TempDatabase::new();
        let mut store = TaskStore::open(database.path()).expect("open store");
        create_task_with_region_alias(
            &mut store,
            &database,
            task_id,
            "region-native-invalid",
            "download",
            "dem_download",
            "2026-08-17T00:00:00Z",
            Some(region_id_alias),
        );
        complete(&mut store, task_id, "2026-08-17T00:01:00Z");

        assert_eq!(
            resolve(&store, "region-native-invalid"),
            None,
            "native alias mismatch must not resolve {task_id}"
        );
        assert!(
            store
                .archive_task(&StoreArchiveTaskRequest {
                    task_id: task_id.to_owned(),
                    expected_revision: 1,
                    archived_at: "2026-08-17T00:02:00Z".to_owned(),
                })
                .is_err(),
            "native alias mismatch must not materialize {task_id}"
        );
    }
}

#[test]
fn resolver_rejects_legacy_dem_prepare_without_payload_region_alias() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    create_task_with_region_alias(
        &mut store,
        &database,
        "legacy-missing-region-alias",
        "region-legacy-missing",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:00:00Z",
        None,
    );
    complete(
        &mut store,
        "legacy-missing-region-alias",
        "2026-08-17T00:01:00Z",
    );

    assert_eq!(resolve(&store, "region-legacy-missing"), None);
    assert!(store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "legacy-missing-region-alias".to_owned(),
            expected_revision: 1,
            archived_at: "2026-08-17T00:02:00Z".to_owned(),
        })
        .is_err());
}

#[test]
fn resolver_filters_authority_and_orders_current_and_archived_sources_deterministically() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");

    for (task_id, region, phase, worker, completed_at, archived_at) in [
        (
            "archived-valid",
            "region-dem",
            "execute",
            "dem_prepare_sarscape",
            "2026-08-17T00:01:00Z",
            Some("2026-08-17T00:12:00Z"),
        ),
        (
            "wrong-region",
            "region-other",
            "execute",
            "dem_prepare_sarscape",
            "2026-08-17T00:02:00Z",
            Some("2026-08-17T00:20:00Z"),
        ),
        (
            "wrong-phase",
            "region-dem",
            "download",
            "dem_prepare_sarscape",
            "2026-08-17T00:03:00Z",
            Some("2026-08-17T00:21:00Z"),
        ),
        (
            "wrong-worker",
            "region-dem",
            "execute",
            "dem_download",
            "2026-08-17T00:03:30Z",
            Some("2026-08-17T00:21:30Z"),
        ),
        (
            "tombstoned-newest",
            "region-dem",
            "execute",
            "dem_prepare_sarscape",
            "2026-08-17T00:04:00Z",
            Some("2026-08-17T00:22:00Z"),
        ),
        (
            "current-older",
            "region-dem",
            "execute",
            "dem_prepare_sarscape",
            "2026-08-17T00:11:00Z",
            None,
        ),
    ] {
        create_task(
            &mut store,
            &database,
            task_id,
            region,
            phase,
            worker,
            "2026-08-17T00:00:00Z",
        );
        complete(&mut store, task_id, completed_at);
        if let Some(archived_at) = archived_at {
            archive(&mut store, task_id, archived_at);
        }
    }
    store
        .delete_history(&StoreDeleteHistoryRequest {
            task_id: "tombstoned-newest".to_owned(),
            deleted_at: "2026-08-17T00:23:00Z".to_owned(),
        })
        .expect("tombstone history");
    create_task_with_kind(
        &mut store,
        &database,
        "wrong-kind",
        "orbit",
        "region-dem",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:00:00Z",
    );
    complete(&mut store, "wrong-kind", "2026-08-17T00:05:00Z");
    archive(&mut store, "wrong-kind", "2026-08-17T00:24:00Z");
    create_task(
        &mut store,
        &database,
        "queued-not-completed",
        "region-dem",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:30:00Z",
    );

    for (task_id, dataset, provider, resource_id, capability_id, archive_minute) in [
        (
            "wrong-dataset",
            "COP90",
            PROVIDER,
            RESOURCE_ID,
            CAPABILITY_ID,
            25,
        ),
        (
            "wrong-provider",
            DATASET,
            "OPENTOPOGRAPHY",
            RESOURCE_ID,
            CAPABILITY_ID,
            26,
        ),
        (
            "wrong-resource",
            DATASET,
            PROVIDER,
            "dem.cop90.v1",
            CAPABILITY_ID,
            27,
        ),
        (
            "wrong-capability",
            DATASET,
            PROVIDER,
            RESOURCE_ID,
            "capability.dem.wrong.v1",
            28,
        ),
    ] {
        let output_root = database.path().parent().unwrap().join(task_id);
        let input_path = output_root.join("source.tif");
        create_task_with_authority(
            &mut store,
            &database,
            task_id,
            "dem",
            "region-dem",
            "execute",
            "dem_prepare_sarscape",
            "2026-08-17T00:00:00Z",
            dataset,
            provider,
            resource_id,
            capability_id,
            &input_path,
            &output_root,
        );
        complete(&mut store, task_id, "2026-08-17T00:06:00Z");
        archive(
            &mut store,
            task_id,
            &format!("2026-08-17T00:{archive_minute:02}:00Z"),
        );
    }

    for (task_id, dataset, provider, resource_id, capability_id) in [
        (
            "live-wrong-dataset",
            "COP90",
            PROVIDER,
            RESOURCE_ID,
            CAPABILITY_ID,
        ),
        (
            "live-wrong-provider",
            DATASET,
            "OPENTOPOGRAPHY",
            RESOURCE_ID,
            CAPABILITY_ID,
        ),
        (
            "live-wrong-resource",
            DATASET,
            PROVIDER,
            "dem.cop90.v1",
            CAPABILITY_ID,
        ),
        (
            "live-wrong-capability",
            DATASET,
            PROVIDER,
            RESOURCE_ID,
            "capability.dem.wrong.v1",
        ),
    ] {
        let output_root = database.path().parent().unwrap().join(task_id);
        let input_path = output_root.join("source.tif");
        create_task_with_authority(
            &mut store,
            &database,
            task_id,
            "dem",
            "region-dem",
            "execute",
            "dem_prepare_sarscape",
            "2026-08-17T00:00:00Z",
            dataset,
            provider,
            resource_id,
            capability_id,
            &input_path,
            &output_root,
        );
        complete(&mut store, task_id, "2026-08-17T00:50:00Z");
    }
    let oversized_output = database
        .path()
        .parent()
        .unwrap()
        .join("live-oversized-path");
    create_task_with_authority(
        &mut store,
        &database,
        "live-oversized-path",
        "dem",
        "region-dem",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:00:00Z",
        DATASET,
        PROVIDER,
        RESOURCE_ID,
        CAPABILITY_ID,
        &PathBuf::from("X".repeat(4_097)),
        &oversized_output,
    );
    complete(&mut store, "live-oversized-path", "2026-08-17T00:51:00Z");

    assert_eq!(
        resolve(&store, "region-dem").unwrap().task_id,
        "archived-valid",
        "newer wrong identity rows are filtered before ordering"
    );

    create_task(
        &mut store,
        &database,
        "current-newest",
        "region-dem",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:00:00Z",
    );
    complete(&mut store, "current-newest", "2026-08-17T00:13:00Z");
    assert_eq!(
        resolve(&store, "region-dem").unwrap().task_id,
        "current-newest",
        "newer live completion beats the archive"
    );

    create_task(
        &mut store,
        &database,
        "aaa-current-tie",
        "region-tie",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:00:00Z",
    );
    complete(&mut store, "aaa-current-tie", "2026-08-17T00:40:00Z");
    create_task(
        &mut store,
        &database,
        "zzz-archive-tie",
        "region-tie",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:00:00Z",
    );
    complete(&mut store, "zzz-archive-tie", "2026-08-17T00:39:00Z");
    archive(&mut store, "zzz-archive-tie", "2026-08-17T00:40:00Z");
    assert_eq!(
        resolve(&store, "region-tie").unwrap().task_id,
        "aaa-current-tie",
        "equal epochs use ascending task_id across UNION branches"
    );

    assert_eq!(resolve(&store, "region-missing"), None);
}

#[test]
fn resolver_rejects_invalid_region_identity_before_sql() {
    let database = TempDatabase::new();
    let store = TaskStore::open(database.path()).expect("open store");
    for invalid in ["", " region", "region\n", &"x".repeat(129)] {
        assert!(store
            .resolve_dem_conversion_source(&StoreDemConversionSourceRequest {
                region_id: invalid.to_owned(),
                ..source_request("region")
            })
            .is_err());
    }
    for field in ["dataset", "provider", "resource_id", "capability_id"] {
        let mut request = serde_json::to_value(source_request("region")).unwrap();
        request[field] = Value::String("".to_owned());
        let request: StoreDemConversionSourceRequest = serde_json::from_value(request).unwrap();
        assert!(store.resolve_dem_conversion_source(&request).is_err());
    }
}

#[test]
fn failed_version_ten_authority_creation_rolls_back_without_schema_promotion() {
    let database = TempDatabase::new();
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             DROP TABLE task_history_dem_source_authority;
             DROP TABLE dem_source_identity;
             CREATE TABLE task_history_dem_source_authority(blocker INTEGER);
             PRAGMA user_version = 9;",
        )
        .unwrap();
    drop(connection);

    assert!(TaskStore::open_existing(database.path()).is_err());
    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(
        version, 9,
        "failed v10 migration cannot promote user_version"
    );
}

#[test]
fn archive_rejects_invalid_source_paths_without_partial_history_rows() {
    let database = TempDatabase::new();
    let output_root = database.path().parent().unwrap().join("bounded-output");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_task_with_authority(
        &mut store,
        &database,
        "invalid-source-path",
        "dem",
        "region-dem",
        "execute",
        "dem_prepare_sarscape",
        "2026-08-17T00:00:00Z",
        DATASET,
        PROVIDER,
        RESOURCE_ID,
        CAPABILITY_ID,
        &PathBuf::from("X".repeat(4_097)),
        &output_root,
    );
    complete(&mut store, "invalid-source-path", "2026-08-17T00:01:00Z");
    assert!(store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "invalid-source-path".to_owned(),
            expected_revision: 1,
            archived_at: "2026-08-17T00:02:00Z".to_owned(),
        })
        .is_err());
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    for table in [
        "task_history",
        "task_history_summary",
        "task_history_dem_source_authority",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "invalid authority must roll back {table}");
    }
}
