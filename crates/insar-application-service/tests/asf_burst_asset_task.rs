use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, BurstAppendAssetTaskRequest, BurstAssetPolicy,
    BurstCreateAssetTaskRequest, BurstCreateHistoricalAssetTaskRequest, UnifiedCreateTaskOptions,
    ASF_BURST_COLLECTION_PROFILE_ID,
};
use insar_core::{
    stable_asset_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse, AssetRole,
    CatalogSearchQuery, CatalogSearchResponse, ContractError, DownloadItemProgress,
    DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenSearchBatch, ProviderAsset,
    ProviderResult, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::{decode_v2_only_snapshot, StoredTaskSnapshot, TaskStore};
use rusqlite::Connection;
use serde_json::{json, Map};

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-burst-asset-task-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

struct FixtureAsf;

impl LiveAsfSearch for FixtureAsf {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(fixture_error())
    }
}

struct FixtureCatalog;

impl LiveCatalogSearch for FixtureCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        Err(fixture_error())
    }
}

struct FixtureWorker;

impl DownloadWorker for FixtureWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root.join(work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-26T01:01:00Z".to_owned(),
        }
    }
}

struct BlockingWorker;

impl DownloadWorker for BlockingWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        while !control.is_cancelled() {
            thread::sleep(Duration::from_millis(5));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root.join(work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-26T01:05:00Z".to_owned(),
        }
    }
}

fn fixture_error() -> ContractError {
    ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap()
}

fn asset(parent_id: &str, role: AssetRole) -> ProviderAsset {
    let (suffix, media_type, size_bytes) = match role {
        AssetRole::Primary => ("measurement.tiff", "image/tiff", Some(1_024)),
        AssetRole::Metadata => ("metadata.xml", "application/xml", None),
        _ => unreachable!(),
    };
    let asset_key = format!("{parent_id}:{suffix}");
    ProviderAsset {
        stable_asset_key: Some(stable_asset_key(parent_id, &asset_key).unwrap()),
        asset_key: Some(asset_key),
        access_descriptor: None,
        role,
        native_roles: vec!["USE SERVICE API".to_owned()],
        url: Some(format!(
            "https://sentinel1-burst.asf.alaska.edu/{parent_id}/{suffix}"
        )),
        file_name: Some(suffix.to_owned()),
        media_type: Some(media_type.to_owned()),
        size_bytes,
    }
}

fn product(parent_id: &str) -> SnapshotItem {
    let assets = vec![
        asset(parent_id, AssetRole::Primary),
        asset(parent_id, AssetRole::Metadata),
    ];
    SnapshotItem {
        item_id: parent_id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: parent_id.to_owned(),
        acquisition_datetime: Some("2026-08-20T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: json!({
            "provider": "asf.catalog",
            "dataset": "SLC-BURST",
            "product_family": "BURST",
            "assets": assets
        })
        .as_object()
        .unwrap()
        .clone(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed(database: &TempStore, candidates: Vec<SnapshotItem>) {
    let mut store = TaskStore::open(&database.0).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "burst-source-batch".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: "2026-08-26T00:00:00Z".to_owned(),
                aoi_name: "Burst AOI".to_owned(),
                aoi_geojson: None,
                query: json!({"dataset": "SLC-BURST"}).as_object().unwrap().clone(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: "burst-source-batch".to_owned(),
                checked_item_ids: vec!["BURST-A".to_owned(), "BURST-B".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
}

fn activate_unrelated_main_workbench(database: &TempStore) {
    let mut unrelated = TaskStore::open(&database.0).unwrap();
    unrelated
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "unrelated-main-batch".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: "2026-08-26T01:03:00Z".to_owned(),
                aoi_name: "Other".to_owned(),
                aoi_geojson: None,
                query: json!({"dataset": "SLC-BURST"}).as_object().unwrap().clone(),
                search_stats: None,
                candidates: vec![product("OTHER")],
            },
            workbench: SearchWorkbenchState {
                batch_id: "unrelated-main-batch".to_owned(),
                checked_item_ids: vec!["OTHER".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
}

fn service(database: &TempStore) -> ApplicationService {
    service_with_worker(database, Arc::new(FixtureWorker))
}

fn service_with_worker(
    database: &TempStore,
    worker: Arc<dyn DownloadWorker>,
) -> ApplicationService {
    ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(FixtureCatalog),
        worker,
    )
}

fn request(task_id: &str, policy: BurstAssetPolicy) -> BurstCreateAssetTaskRequest {
    BurstCreateAssetTaskRequest {
        request_id: format!("{task_id}-request"),
        source_batch_id: "burst-source-batch".to_owned(),
        selected_product_ids: vec!["BURST-B".to_owned(), "BURST-A".to_owned()],
        asset_policy: policy,
        expected_workbench_revision: 0,
        task_id: task_id.to_owned(),
        output_root: std::env::temp_dir().join("burst-asset-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-08-26T01:00:00Z".to_owned(),
    }
}

fn wait_for_running_revision(database: &TempStore, task_id: &str) -> u64 {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let connection = Connection::open(&database.0).unwrap();
        let (status, revision) = connection
            .query_row(
                "SELECT status, revision FROM task_runtime WHERE task_id = ?1",
                [task_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        if status == "running" {
            return u64::try_from(revision).unwrap();
        }
        assert!(std::time::Instant::now() < deadline, "task did not start");
        thread::sleep(Duration::from_millis(10));
    }
}

fn compact(database: &TempStore, task_id: &str) -> insar_store::CompactTaskSnapshotV2 {
    let connection = Connection::open(&database.0).unwrap();
    let snapshot: String = connection
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    match decode_v2_only_snapshot(&snapshot).unwrap() {
        StoredTaskSnapshot::CompactV2(compact) => compact,
        other => panic!("expected compact snapshot, got {other:?}"),
    }
}

#[test]
fn burst_policies_freeze_exact_assets_without_replacing_the_source_batch() {
    let database = TempStore::new("policies");
    seed(&database, vec![product("BURST-A"), product("BURST-B")]);
    let service = service(&database);

    for (task_id, policy, expected_roles) in [
        (
            "burst-all",
            BurstAssetPolicy::All,
            vec!["primary", "metadata", "primary", "metadata"],
        ),
        (
            "burst-primary",
            BurstAssetPolicy::Primary,
            vec!["primary", "primary"],
        ),
        (
            "burst-metadata",
            BurstAssetPolicy::Metadata,
            vec!["metadata", "metadata"],
        ),
    ] {
        service
            .create_burst_asset_task(&request(task_id, policy))
            .unwrap();
        let frozen = compact(&database, task_id);
        assert_eq!(frozen.batch_id, "burst-source-batch");
        assert_eq!(frozen.source_item_ids, ["BURST-A", "BURST-B"]);
        assert_eq!(
            frozen
                .selected_products
                .iter()
                .map(|item| item.metadata["asset_role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected_roles
        );
        assert_eq!(
            frozen
                .selected_products
                .iter()
                .map(|item| item.metadata["source_item_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            match policy {
                BurstAssetPolicy::All => vec!["BURST-A", "BURST-A", "BURST-B", "BURST-B"],
                BurstAssetPolicy::Primary | BurstAssetPolicy::Metadata => {
                    vec!["BURST-A", "BURST-B"]
                }
            }
        );
        for item in &frozen.selected_products {
            assert_eq!(item.item_id, item.metadata["stable_asset_key"]);
            assert_eq!(item.metadata["burst_asset_policy"], policy.as_str());
            assert!(item.metadata["official_href"]
                .as_str()
                .unwrap()
                .starts_with("https://sentinel1-burst.asf.alaska.edu/"));
            assert_eq!(item.payload["burst_asset"], true);
            assert_eq!(
                item.execution_policy
                    .as_ref()
                    .and_then(|policy| policy.collection_profile_id.as_deref()),
                Some("asf.sentinel-1.burst.v1")
            );
            assert_eq!(ASF_BURST_COLLECTION_PROFILE_ID, "asf.sentinel-1.burst.v1");
        }
    }

    service.shutdown().unwrap();
}

#[test]
fn malformed_frozen_burst_assets_fail_closed_without_creating_a_task() {
    for label in [
        "missing-primary",
        "missing-role",
        "duplicate-role",
        "wrong-extension",
        "wrong-media",
        "non-https",
        "untrusted-host",
        "query-bearing",
        "fragment-bearing",
        "missing-stable-key",
        "missing-file-name",
        "missing-url",
        "negative-size",
        "overflow-size",
    ] {
        let database = TempStore::new(label);
        let mut malformed = product("BURST-A");
        let assets = malformed.metadata["assets"].as_array_mut().unwrap();
        match label {
            "missing-primary" => {
                assets.remove(0);
            }
            "missing-role" => {
                assets[0].as_object_mut().unwrap().remove("role");
            }
            "duplicate-role" => assets[1]["role"] = json!("primary"),
            "wrong-extension" => {
                assets[0]["file_name"] = json!("measurement.xml");
                assets[0]["url"] =
                    json!("https://sentinel1-burst.asf.alaska.edu/BURST-A/measurement.xml");
            }
            "wrong-media" => assets[0]["media_type"] = json!("application/xml"),
            "non-https" => {
                assets[0]["url"] =
                    json!("http://sentinel1-burst.asf.alaska.edu/BURST-A/measurement.tiff")
            }
            "untrusted-host" => {
                assets[0]["url"] = json!("https://example.invalid/BURST-A/measurement.tiff")
            }
            "query-bearing" => {
                assets[0]["url"] =
                    json!("https://sentinel1-burst.asf.alaska.edu/BURST-A/measurement.tiff?token=x")
            }
            "fragment-bearing" => {
                assets[0]["url"] = json!(
                    "https://sentinel1-burst.asf.alaska.edu/BURST-A/measurement.tiff#fragment"
                )
            }
            "missing-stable-key" => {
                assets[0]
                    .as_object_mut()
                    .unwrap()
                    .remove("stable_asset_key");
            }
            "missing-file-name" => {
                assets[0].as_object_mut().unwrap().remove("file_name");
            }
            "missing-url" => {
                assets[0].as_object_mut().unwrap().remove("url");
            }
            "negative-size" => assets[0]["size_bytes"] = json!(-1),
            "overflow-size" => assets[0]["size_bytes"] = json!(u64::MAX),
            _ => unreachable!(),
        }
        seed(&database, vec![malformed, product("BURST-B")]);
        let service = service(&database);
        let task_id = format!("burst-invalid-{label}");

        let error = service
            .create_burst_asset_task(&request(&task_id, BurstAssetPolicy::All))
            .unwrap_err();
        assert_eq!(
            error.code(),
            "ASF_BURST_FROZEN_ASSET_EVIDENCE_INVALID",
            "{label}: {error:?}"
        );
        let exists: bool = Connection::open(&database.0)
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1)",
                [&task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists, "{label} must not create any task row");
        service.shutdown().unwrap();
    }
}

#[test]
fn typed_burst_append_reuses_the_frozen_policy_for_new_source_products() {
    let database = TempStore::new("append");
    seed(&database, vec![product("BURST-A"), product("BURST-B")]);
    let service = service_with_worker(&database, Arc::new(BlockingWorker));
    let mut create = request("burst-append", BurstAssetPolicy::All);
    create.selected_product_ids = vec!["BURST-A".to_owned()];
    service.create_burst_asset_task(&create).unwrap();
    let expected_task_revision = wait_for_running_revision(&database, "burst-append");
    activate_unrelated_main_workbench(&database);

    service
        .append_burst_asset_task(&BurstAppendAssetTaskRequest {
            request_id: "burst-append-request-b".to_owned(),
            task_id: "burst-append".to_owned(),
            selected_product_ids: vec!["BURST-B".to_owned()],
            expected_task_revision,
            event_id: "burst-append:event-b".to_owned(),
            occurred_at: "2026-08-26T01:02:00Z".to_owned(),
        })
        .unwrap();

    let task = TaskStore::open(&database.0)
        .unwrap()
        .load_scheduler_task("burst-append")
        .unwrap()
        .unwrap();
    assert_eq!(task.snapshot.batch_id, "burst-source-batch");
    assert_eq!(task.snapshot.candidates.len(), 4);
    assert_eq!(
        task.snapshot.candidates[2..]
            .iter()
            .map(|item| (
                item.metadata["source_item_id"].as_str().unwrap(),
                item.metadata["asset_role"].as_str().unwrap(),
                item.metadata["burst_asset_policy"].as_str().unwrap(),
            ))
            .collect::<Vec<_>>(),
        [
            ("BURST-B", "primary", "all"),
            ("BURST-B", "metadata", "all"),
        ]
    );
    service.shutdown().unwrap();
}

#[test]
fn terminal_burst_successor_uses_the_original_batch_and_policy_not_the_main_workbench() {
    let database = TempStore::new("historical-successor");
    seed(&database, vec![product("BURST-A"), product("BURST-B")]);
    let service = service(&database);
    let mut create = request("burst-terminal-source", BurstAssetPolicy::Metadata);
    create.selected_product_ids = vec!["BURST-A".to_owned()];
    service.create_burst_asset_task(&create).unwrap();

    let (source_status, source_revision) = {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let row = Connection::open(&database.0)
                .unwrap()
                .query_row(
                    "SELECT status, revision FROM task_runtime WHERE task_id = 'burst-terminal-source'",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap();
            if matches!(row.0.as_str(), "completed" | "failed" | "cancelled") {
                break (row.0, u64::try_from(row.1).unwrap());
            }
            assert!(
                std::time::Instant::now() < deadline,
                "source task did not terminate"
            );
            thread::sleep(Duration::from_millis(10));
        }
    };

    // Replace the main Workbench to prove the historical successor reads the
    // immutable source SearchBatch by the source task's batch_id.
    activate_unrelated_main_workbench(&database);

    service
        .create_historical_burst_asset_task(&BurstCreateHistoricalAssetTaskRequest {
            request_id: "burst-historical-successor-request".to_owned(),
            source_task_id: "burst-terminal-source".to_owned(),
            expected_source_task_revision: source_revision,
            selected_product_ids: vec!["BURST-B".to_owned()],
            new_task_id: "burst-historical-successor".to_owned(),
            output_root: std::env::temp_dir().join("burst-historical-successor-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-26T01:04:00Z".to_owned(),
        })
        .unwrap();

    let successor = compact(&database, "burst-historical-successor");
    assert_eq!(successor.batch_id, "burst-source-batch");
    assert_eq!(successor.source_item_ids, ["BURST-B"]);
    assert_eq!(successor.selected_products.len(), 1);
    assert_eq!(
        successor.selected_products[0].metadata["asset_role"],
        "metadata"
    );
    assert_eq!(
        successor.selected_products[0].metadata["burst_asset_policy"],
        "metadata"
    );
    let source_after = Connection::open(&database.0)
        .unwrap()
        .query_row(
            "SELECT status, revision FROM task_runtime WHERE task_id = 'burst-terminal-source'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(source_after, (source_status, source_revision as i64));
    service.shutdown().unwrap();
}

/// A unified-search frozen SLC-BURST parent (no download_url, assets in
/// metadata) must expand into per-asset task items whose payloads carry the
/// download address the verified burst worker reads (DL004 defect,
/// 2026-10-31). The recording worker stands in for the production worker and
/// records the exact frozen item it is handed.
struct RecordingBurstWorker {
    executed: Arc<Mutex<Vec<SnapshotItem>>>,
}

impl DownloadWorker for RecordingBurstWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.executed.lock().unwrap().push(work.item.clone());
        let destination = work.task.output_root.join(&work.item.display_name);
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-10-31T01:01:00Z".to_owned(),
        }
    }
}

/// The parent shape the unified search freeze produces for one burst:
/// `map_sar_product` of the ASF burst mapper — assets in metadata, only
/// `file_size_remote` in the payload, `download_url` absent (DL004 defect).
fn unified_burst_parent() -> SnapshotItem {
    let native_id = "S1_000001_IW2_20260820T000000_VV_A1B2-BURST";
    let product_key = format!("ASF::SLC-BURST::{native_id}");
    let primary_key =
        stable_asset_key(&product_key, &format!("{native_id}:measurement.tiff")).unwrap();
    let metadata_key =
        stable_asset_key(&product_key, &format!("{native_id}:metadata.xml")).unwrap();
    let assets = json!([
        {
            "asset_key": format!("{native_id}:measurement.tiff"),
            "stable_asset_key": primary_key,
            "access_descriptor": null,
            "role": "primary",
            "native_roles": ["USE SERVICE API"],
            "url": format!("https://sentinel1-burst.asf.alaska.edu/{native_id}/measurement.tiff"),
            "file_name": "measurement.tiff",
            "media_type": "image/tiff",
            "size_bytes": 1_024
        },
        {
            "asset_key": format!("{native_id}:metadata.xml"),
            "stable_asset_key": metadata_key,
            "access_descriptor": null,
            "role": "metadata",
            "native_roles": ["USE SERVICE API"],
            "url": format!("https://sentinel1-burst.asf.alaska.edu/{native_id}/metadata.xml"),
            "file_name": "metadata.xml",
            "media_type": "application/xml",
            "size_bytes": null
        }
    ]);
    SnapshotItem {
        item_id: product_key.clone(),
        provider_id: "asf.catalog".to_owned(),
        display_name: native_id.to_owned(),
        acquisition_datetime: Some("2026-08-20T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: json!({
            "product_key": product_key,
            "provider": "asf.catalog",
            "dataset": "SLC-BURST",
            "product_id": native_id,
            "product_family": "BURST",
            "provider_product_key": format!("ASF::SLC-BURST::{native_id}"),
            "provider_product_id": "G12345-ASF",
            "scene_id": native_id,
            "platform": "SENTINEL-1A",
            "instrument": "C-SAR",
            "processing_level": "L1",
            "beam_mode": "IW",
            "beam_swath": "IW2",
            "polarization": "VV",
            "flight_direction": "ASCENDING",
            "look_direction": "RIGHT",
            "relative_orbit": 141,
            "absolute_orbit": 57142,
            "path": 141,
            "identity_rule_id": "asf.sentinel-1.burst.v1",
            "stable_asset_keys": [primary_key, metadata_key],
            "assets": assets,
            "burst": {
                "burst_id": null,
                "full_burst_id": "141_301495_IW2",
                "relative_burst_id": 301495,
                "absolute_burst_id": 5714273,
                "subswath": "IW2",
                "burst_index": 0
            },
            "native_metadata": {
                "cmr_granule_concept_id": "G12345-ASF",
                "cmr_native_id": native_id,
                "raw_authority": {},
                "collection": {
                    "concept_id": "C2709161906-ASF",
                    "short_name": "SENTINEL-1_BURSTS",
                    "version": "1",
                    "provider_id": "ASF"
                },
                "burst": {
                    "full_id": "141_301495_IW2",
                    "relative_id": 301495,
                    "absolute_id": 5714273,
                    "index": 0,
                    "subswath": "IW2",
                    "byte_length": 1024
                },
                "scope": {
                    "beam_mode": "IW",
                    "processing_level": "L1",
                    "processing_type": "BURST",
                    "path": 141,
                    "flight_direction": "ASCENDING",
                    "platform": "SENTINEL-1A",
                    "instrument": "C-SAR",
                    "look_direction": "RIGHT"
                },
                "browse": null,
                "preview_available": false
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        payload: Map::from_iter([("file_size_remote".to_owned(), json!(1_024))]),
        asset_access: None,
        execution_policy: None,
    }
}

#[test]
fn unified_create_task_expands_a_frozen_burst_parent_into_worker_download_items() {
    let database = TempStore::new("unified-burst-dl004");
    let parent = unified_burst_parent();
    let batch_id = "burst-source-batch".to_owned();
    let item_id = parent.item_id.clone();
    {
        let mut store = TaskStore::open(&database.0).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.clone(),
                    provider_id: "asf.catalog".to_owned(),
                    created_at: "2026-10-31T00:00:00Z".to_owned(),
                    aoi_name: "Fairbanks".to_owned(),
                    aoi_geojson: None,
                    query: json!({"dataset": "SLC-BURST"}).as_object().unwrap().clone(),
                    search_stats: None,
                    candidates: vec![parent],
                },
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.clone(),
                    checked_item_ids: vec![item_id.clone()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .unwrap();
    }
    let executed = Arc::new(Mutex::new(Vec::new()));
    let service = service_with_worker(
        &database,
        Arc::new(RecordingBurstWorker {
            executed: Arc::clone(&executed),
        }),
    );

    let task_ref = service
        .create_task(
            batch_id,
            0,
            vec![item_id],
            UnifiedCreateTaskOptions {
                output_root: std::env::temp_dir().join("unified-burst-output"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                include_precise_orbit: None,
            },
        )
        .unwrap();
    assert!(task_ref.created);

    // Wait for the scheduler to hand the frozen items to the worker.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while executed.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    service.shutdown().unwrap();

    let items = executed.lock().unwrap().clone();
    assert_eq!(items.len(), 2, "one burst parent freezes two asset items");
    let by_role = |role: &str| {
        items
            .iter()
            .find(|item| {
                item.payload["burst_asset"] == json!(true)
                    && item.payload["asset_role"] == json!(role)
            })
            .unwrap_or_else(|| panic!("missing {role} asset item"))
    };
    for (role, file_name) in [
        ("primary", "measurement.tiff"),
        ("metadata", "metadata.xml"),
    ] {
        let item = by_role(role);
        // The DL004 defect: the worker reads payload.download_url / file_name.
        assert_eq!(
            item.payload["download_url"].as_str().unwrap(),
            format!("https://sentinel1-burst.asf.alaska.edu/S1_000001_IW2_20260820T000000_VV_A1B2-BURST/{file_name}"),
            "{role} asset item carries the official download address"
        );
        assert_eq!(item.payload["file_name"].as_str().unwrap(), file_name);
        assert_eq!(item.display_name, file_name);
        assert_eq!(item.provider_id, "asf.catalog");
        assert_eq!(item.metadata["dataset"], json!("SLC-BURST"));
    }
    let primary = by_role("primary");
    assert_eq!(primary.payload["file_size_remote"], json!(1_024));
    assert_eq!(
        primary.execution_policy.as_ref().unwrap().policy_id,
        "asf.sentinel-1.burst.asset.v1"
    );
}
