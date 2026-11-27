//! One-shot, credential-free acceptance for a complete Landsat C2 L2 asset set.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, LandsatCreateBundleTaskRequest,
    LiveLandsatAssetManifest,
};
use insar_core::{
    provider_product_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery,
    CatalogSearchResponse, ContractError, DownloadItemStatus, DownloadOutcome, FrozenSearchBatch,
    ProviderNetworkPolicy, ProviderResult, SearchWorkbenchState, SnapshotItem,
    StoreSearchBatchRequest,
};
use insar_download::{part_path, ProductionDownloadWorker, TransferOptions};
use insar_provider::catalog::{
    MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_FILENAME_RULE_ID, MPC_LANDSAT_IDENTITY_RULE_ID,
    MPC_LANDSAT_PROVIDER_ID, MPC_LANDSAT_SCENE_PROFILE_ID,
};
use insar_provider::planetary_computer::{
    PlanetaryComputerError, PlanetaryComputerLandsatClient, PlanetaryComputerLandsatManifest,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

const ENTITY_ID: &str = "LC90130352023085LGN00";
const MPC_ITEM_ID: &str = "LC09_L2SP_013035_20230326_02_T1";
const DISPLAY_ID: &str = "LC09_L2SP_013035_20230326_20230328_02_T1";
const SOURCE_BATCH_ID: &str = "landsat-p4a3-pc-source";
const TASK_ID: &str = "landsat-p4a3-pc-task";
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(45 * 60);

#[test]
#[ignore = "requires explicit authorization for an approximately 819 MB public-network transfer"]
fn landsat_planetary_computer_complete_asset_set_live_acceptance() {
    match run_live() {
        Ok(report) => {
            eprintln!("LANDSAT_P4A3_PC_LIVE outcome=completed");
            eprintln!("credential_input_count=0");
            eprintln!("asset_count={}", report.asset_count);
            eprintln!("expected_bytes={}", report.expected_bytes);
            eprintln!("actual_bytes={}", report.actual_bytes);
            eprintln!("stac_item_requests={}", report.stac_item_requests);
            eprintln!("sas_token_requests={}", report.sas_token_requests);
            eprintln!("asset_head_requests={}", report.asset_head_requests);
            eprintln!("part_observed={}", report.part_observed);
            eprintln!("active_observed={}", report.active_observed);
            eprintln!("size_verified=true");
            eprintln!("atomic_final=true");
            eprintln!("store_restore_completed=true");
            eprintln!("acceptance_workspace_cleaned=true");
        }
        Err(code) => {
            eprintln!("LANDSAT_P4A3_PC_LIVE outcome=stopped");
            eprintln!("code={code}");
            panic!("LANDSAT_P4A3_PC_LIVE_STOPPED");
        }
    }
}

struct LiveReport {
    asset_count: usize,
    expected_bytes: u64,
    actual_bytes: u64,
    stac_item_requests: u64,
    sas_token_requests: u64,
    asset_head_requests: u64,
    part_observed: bool,
    active_observed: bool,
}

/// Owns the caller-designated, previously absent live-acceptance root.
///
/// Successful runs remove the root explicitly after all verification and Store
/// reopen checks. Early returns use `Drop` as a best-effort safety net so a
/// failed network acceptance does not intentionally retain downloaded bytes.
struct LiveAcceptanceWorkspace {
    root: PathBuf,
}

impl LiveAcceptanceWorkspace {
    fn create(root: PathBuf) -> Result<Self, String> {
        if root.exists() {
            return Err("LANDSAT_P4A3_PC_ROOT_OCCUPIED".to_owned());
        }
        fs::create_dir_all(root.join("output"))
            .map_err(|_| "LANDSAT_P4A3_PC_ROOT_UNAVAILABLE".to_owned())?;
        Ok(Self { root })
    }

    fn output_root(&self) -> PathBuf {
        self.root.join("output")
    }

    fn database(&self) -> PathBuf {
        self.root.join("tasks.sqlite3")
    }

    fn cleanup(mut self) -> Result<(), String> {
        let root = std::mem::take(&mut self.root);
        fs::remove_dir_all(&root).map_err(|_| "LANDSAT_P4A3_PC_CLEANUP_FAILED".to_owned())?;
        if root.exists() {
            return Err("LANDSAT_P4A3_PC_CLEANUP_INCOMPLETE".to_owned());
        }
        Ok(())
    }
}

impl Drop for LiveAcceptanceWorkspace {
    fn drop(&mut self) {
        if !self.root.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.root);
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

fn fixture_error() -> ContractError {
    ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap()
}

struct FixedManifest(PlanetaryComputerLandsatManifest);

impl LiveLandsatAssetManifest for FixedManifest {
    fn landsat_asset_manifest(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError> {
        assert_eq!(dataset_alias, MPC_LANDSAT_COLLECTION_ID);
        assert_eq!(entity_id, self.0.entity_id);
        assert_eq!(display_id, self.0.display_id);
        assert!(!cancelled());
        Ok(self.0.clone())
    }
}

fn run_live() -> Result<LiveReport, String> {
    let root = std::env::var_os("INSAR_LANDSAT_P4A3_LIVE_ROOT")
        .map(PathBuf::from)
        .ok_or_else(|| "LANDSAT_P4A3_PC_ROOT_REQUIRED".to_owned())?;
    let workspace = LiveAcceptanceWorkspace::create(root)?;
    let output_root = workspace.output_root();
    let database = workspace.database();

    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::system())
        .map_err(|error| format!("LANDSAT_P4A3_PC_CLIENT_{}", error.code().as_str()))?;
    let manifest = client
        .manifest(MPC_LANDSAT_COLLECTION_ID, MPC_ITEM_ID, DISPLAY_ID, &|| {
            false
        })
        .map_err(|error| {
            eprintln!("manifest_endpoint={}", error.endpoint().as_str());
            eprintln!(
                "manifest_http_status={}",
                error.http_status().map_or(0, u16::from)
            );
            format!("LANDSAT_P4A3_PC_MANIFEST_{}", error.code().as_str())
        })?;
    validate_manifest_bounds(&manifest)?;
    let request_counts = client.request_counts();
    let available = available_disk_space(&output_root)?;
    let required = manifest
        .total_size_bytes
        .checked_add(512 * 1024 * 1024)
        .ok_or_else(|| "LANDSAT_P4A3_PC_SIZE_OVERFLOW".to_owned())?;
    if available < required {
        return Err("LANDSAT_P4A3_PC_DISK_SPACE_INSUFFICIENT".to_owned());
    }

    let (batch, source_item_id) = source_batch()?;
    let mut store =
        TaskStore::open(&database).map_err(|_| "LANDSAT_P4A3_PC_STORE_OPEN_FAILED".to_owned())?;
    store
        .save_search_batch(&StoreSearchBatchRequest {
            workbench: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: vec![source_item_id.clone()],
                page_index: 0,
                page_size: 10,
                revision: 0,
            },
            batch,
            activate_workbench: true,
        })
        .map_err(|_| "LANDSAT_P4A3_PC_SOURCE_FREEZE_FAILED".to_owned())?;
    drop(store);

    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .map_err(|_| "LANDSAT_P4A3_PC_WORKER_FAILED".to_owned())?;
    let service = ApplicationService::with_catalogue_production_business_runtime_and_landsat_assets(
        ApplicationServiceConfig::business_store(&database),
        Arc::new(FixtureAsf),
        Arc::new(FixtureCatalog),
        Arc::new(FixedManifest(manifest.clone())),
        Arc::new(worker),
    );
    let created = service
        .create_landsat_asset_task_legacy(&LandsatCreateBundleTaskRequest {
            request_id: "landsat-p4a3-pc-create".to_owned(),
            source_batch_id: SOURCE_BATCH_ID.to_owned(),
            item_id: source_item_id,
            task_id: TASK_ID.to_owned(),
            output_root: output_root.clone(),
            layout: insar_core::DownloadLayout::Direct,
            concurrency: 4,
            created_at: "2026-07-29T00:00:00Z".to_owned(),
        })
        .map_err(|error| format!("LANDSAT_P4A3_PC_TASK_CREATE_{}", error.code()))?;
    let task = created
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == TASK_ID)
        .ok_or_else(|| "LANDSAT_P4A3_PC_TASK_MISSING".to_owned())?;
    if task.candidates.len() != manifest.assets.len()
        || task.initial_selected_item_ids.len() != manifest.assets.len()
    {
        return Err("LANDSAT_P4A3_PC_TASK_ASSET_SET_INVALID".to_owned());
    }
    let transfer_output_root = task.output_root.clone();
    let serialized = serde_json::to_string(task)
        .map_err(|_| "LANDSAT_P4A3_PC_TASK_SERIALIZE_FAILED".to_owned())?
        .to_ascii_lowercase();
    for forbidden in ["sig=", "x-auth-token", "authorization", "requestsignature"] {
        if serialized.contains(forbidden) {
            return Err("LANDSAT_P4A3_PC_SIGNED_MATERIAL_PERSISTED".to_owned());
        }
    }

    let (part_observed, active_observed) =
        wait_for_completion(&service, &manifest, &transfer_output_root)?;
    if !part_observed {
        return Err("LANDSAT_P4A3_PC_PART_NOT_OBSERVED".to_owned());
    }
    let actual_bytes = verify_outputs(&manifest, &transfer_output_root)?;
    service
        .shutdown()
        .map_err(|_| "LANDSAT_P4A3_PC_SHUTDOWN_FAILED".to_owned())?;

    let store_bytes =
        fs::read(&database).map_err(|_| "LANDSAT_P4A3_PC_STORE_READ_FAILED".to_owned())?;
    if store_bytes
        .windows(4)
        .any(|window| window.eq_ignore_ascii_case(b"sig="))
    {
        return Err("LANDSAT_P4A3_PC_SIGNED_MATERIAL_PERSISTED".to_owned());
    }

    let reopened = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let restored = reopened
        .download_snapshot(TASK_ID)
        .map_err(|_| "LANDSAT_P4A3_PC_STORE_RESTORE_FAILED".to_owned())?;
    if restored.results.len() != manifest.assets.len()
        || restored
            .results
            .iter()
            .any(|result| result.outcome != DownloadOutcome::Completed)
    {
        return Err("LANDSAT_P4A3_PC_STORE_RESTORE_INVALID".to_owned());
    }
    let restored_service = reopened
        .snapshot(true)
        .map_err(|_| "LANDSAT_P4A3_PC_STORE_RESTORE_FAILED".to_owned())?;
    if restored_service
        .restore
        .workbench_batch
        .as_ref()
        .is_none_or(|batch| batch.batch_id != SOURCE_BATCH_ID)
    {
        return Err("LANDSAT_P4A3_PC_SOURCE_WORKBENCH_REPLACED".to_owned());
    }
    reopened
        .shutdown()
        .map_err(|_| "LANDSAT_P4A3_PC_REOPEN_SHUTDOWN_FAILED".to_owned())?;
    drop(reopened);
    drop(service);
    workspace.cleanup()?;

    Ok(LiveReport {
        asset_count: manifest.assets.len(),
        expected_bytes: manifest.total_size_bytes,
        actual_bytes,
        stac_item_requests: request_counts.stac_item,
        sas_token_requests: request_counts.sas_token,
        asset_head_requests: request_counts.asset_head,
        part_observed,
        active_observed,
    })
}

#[test]
fn live_acceptance_workspace_removes_test_only_bytes() {
    fn unique_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "insar-landsat-p4a3-pc-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    let explicit_root = unique_root("explicit");
    let workspace = LiveAcceptanceWorkspace::create(explicit_root.clone()).unwrap();
    fs::write(
        workspace.output_root().join("asset.bin"),
        b"acceptance-only",
    )
    .unwrap();
    workspace.cleanup().unwrap();
    assert!(!explicit_root.exists());

    let early_return_root = unique_root("drop");
    {
        let workspace = LiveAcceptanceWorkspace::create(early_return_root.clone()).unwrap();
        fs::write(workspace.output_root().join("partial.bin"), b"partial").unwrap();
    }
    assert!(!early_return_root.exists());
}

fn validate_manifest_bounds(manifest: &PlanetaryComputerLandsatManifest) -> Result<(), String> {
    if manifest.entity_id != MPC_ITEM_ID
        || manifest.display_id != DISPLAY_ID
        || manifest.processing_level != "l2sp"
        || manifest.assets.len() != 22
        || manifest.total_size_bytes == 0
        || manifest.total_size_bytes > MAX_TOTAL_BYTES
        || manifest
            .assets
            .iter()
            .any(|asset| asset.expected_size_bytes == 0)
    {
        return Err("LANDSAT_P4A3_PC_MANIFEST_INVALID".to_owned());
    }
    Ok(())
}

fn source_batch() -> Result<(FrozenSearchBatch, String), String> {
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.landsat.c2.l2",
        "provider_id": MPC_LANDSAT_PROVIDER_ID,
        "native_collection_id": MPC_LANDSAT_COLLECTION_ID,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[
                [-75.446, 36.3793],
                [-76.9196, 36.3793],
                [-76.9196, 35.5713],
                [-75.446, 35.5713],
                [-75.446, 36.3793]
            ]]
        },
        "result_limit": 10,
        "filters": {
            "datetime": {
                "start": "2023-03-24T00:00:00Z",
                "end": "2023-03-27T00:00:00Z"
            },
            "cloud_cover_lte": 100.0,
            "spacecraft": ["landsat_9"],
            "processing_levels": ["l2sp", "l2sr"]
        }
    }))
    .map_err(|_| "LANDSAT_P4A3_PC_QUERY_INVALID".to_owned())?;
    query
        .validate()
        .map_err(|_| "LANDSAT_P4A3_PC_QUERY_INVALID".to_owned())?;
    let item_id = provider_product_key(
        MPC_LANDSAT_PROVIDER_ID,
        MPC_LANDSAT_COLLECTION_ID,
        MPC_ITEM_ID,
    )
    .map_err(|_| "LANDSAT_P4A3_PC_IDENTITY_INVALID".to_owned())?;
    let item = SnapshotItem {
        item_id: item_id.clone(),
        provider_id: MPC_LANDSAT_PROVIDER_ID.to_owned(),
        display_name: DISPLAY_ID.to_owned(),
        acquisition_datetime: Some("2023-03-26T00:00:00Z".to_owned()),
        footprint_geojson: query.aoi_geojson.clone(),
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                Value::String("optical.landsat.c2.l2".to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String(MPC_LANDSAT_COLLECTION_ID.to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(MPC_LANDSAT_SCENE_PROFILE_ID.to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String(MPC_LANDSAT_IDENTITY_RULE_ID.to_owned()),
            ),
            (
                "filename_rule_id".to_owned(),
                Value::String(MPC_LANDSAT_FILENAME_RULE_ID.to_owned()),
            ),
            (
                "delivery_model".to_owned(),
                Value::String("archive_first".to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(MPC_ITEM_ID.to_owned()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(item_id.clone()),
            ),
            (
                "source_kind".to_owned(),
                Value::String("landsat".to_owned()),
            ),
            (
                "product_kind".to_owned(),
                Value::String("optical_scene".to_owned()),
            ),
            (
                "properties".to_owned(),
                json!({
                    "entity_id": MPC_ITEM_ID,
                    "landsat_scene_id": ENTITY_ID,
                    "landsat_product_id": DISPLAY_ID,
                    "dataset_alias": MPC_LANDSAT_COLLECTION_ID,
                    "processing_level": "l2sp"
                }),
            ),
        ]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    let query = serde_json::to_value(&query)
        .map_err(|_| "LANDSAT_P4A3_PC_QUERY_INVALID".to_owned())?
        .as_object()
        .cloned()
        .ok_or_else(|| "LANDSAT_P4A3_PC_QUERY_INVALID".to_owned())?;
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: SOURCE_BATCH_ID.to_owned(),
        provider_id: MPC_LANDSAT_PROVIDER_ID.to_owned(),
        created_at: "2026-07-29T00:00:00Z".to_owned(),
        aoi_name: "P4A-3 public Landsat acceptance AOI".to_owned(),
        aoi_geojson: Some(item.footprint_geojson.clone().unwrap()),
        query,
        search_stats: None,
        candidates: vec![item],
    };
    batch
        .validate()
        .map_err(|_| "LANDSAT_P4A3_PC_SOURCE_BATCH_INVALID".to_owned())?;
    Ok((batch, item_id))
}

fn wait_for_completion(
    service: &ApplicationService,
    manifest: &PlanetaryComputerLandsatManifest,
    output_root: &Path,
) -> Result<(bool, bool), String> {
    let final_paths = manifest
        .assets
        .iter()
        .map(|asset| output_root.join(&asset.file_name))
        .collect::<Vec<_>>();
    let deadline = Instant::now() + TRANSFER_TIMEOUT;
    let mut last_report = Instant::now();
    let mut part_observed = false;
    let mut active_observed = false;
    loop {
        part_observed |= final_paths.iter().any(|path| part_path(path).is_file());
        let snapshot = service
            .download_snapshot(TASK_ID)
            .map_err(|_| "LANDSAT_P4A3_PC_TASK_SNAPSHOT_FAILED".to_owned())?;
        active_observed |= snapshot
            .progress
            .iter()
            .any(|progress| progress.status == DownloadItemStatus::Active);
        if let Some(result) = snapshot.results.iter().find(|result| {
            !matches!(
                result.outcome,
                DownloadOutcome::Completed | DownloadOutcome::Skipped
            )
        }) {
            eprintln!("transfer_failed_item={}", result.item_id);
            eprintln!("transfer_failed_bytes={}", result.bytes_written);
            if let Some(failure) = &result.failure {
                eprintln!("transfer_failure_code={}", failure.code);
                eprintln!("transfer_failure_retryable={}", failure.retryable);
            }
            return Err("LANDSAT_P4A3_PC_TRANSFER_FAILED".to_owned());
        }
        if snapshot.results.len() == manifest.assets.len() {
            if snapshot
                .results
                .iter()
                .any(|result| result.outcome != DownloadOutcome::Completed)
            {
                return Err("LANDSAT_P4A3_PC_TRANSFER_NOT_COMPLETED".to_owned());
            }
            return Ok((part_observed, active_observed));
        }
        if Instant::now() >= deadline {
            return Err("LANDSAT_P4A3_PC_TRANSFER_TIMEOUT".to_owned());
        }
        if last_report.elapsed() >= Duration::from_secs(10) {
            let present_bytes = final_paths
                .iter()
                .filter_map(|path| fs::metadata(path).ok().map(|metadata| metadata.len()))
                .sum::<u64>();
            eprintln!(
                "LANDSAT_P4A3_PC_LIVE progress completed={} final_bytes={}",
                snapshot.results.len(),
                present_bytes
            );
            last_report = Instant::now();
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn verify_outputs(
    manifest: &PlanetaryComputerLandsatManifest,
    output_root: &Path,
) -> Result<u64, String> {
    let mut actual_total = 0_u64;
    for asset in &manifest.assets {
        let final_path = output_root.join(&asset.file_name);
        let actual = fs::metadata(&final_path)
            .map_err(|_| "LANDSAT_P4A3_PC_FINAL_FILE_MISSING".to_owned())?
            .len();
        if actual != asset.expected_size_bytes || part_path(&final_path).exists() {
            return Err("LANDSAT_P4A3_PC_SIZE_OR_PROMOTION_INVALID".to_owned());
        }
        actual_total = actual_total
            .checked_add(actual)
            .ok_or_else(|| "LANDSAT_P4A3_PC_SIZE_OVERFLOW".to_owned())?;
    }
    if actual_total != manifest.total_size_bytes {
        return Err("LANDSAT_P4A3_PC_TOTAL_SIZE_INVALID".to_owned());
    }
    Ok(actual_total)
}

#[cfg(windows)]
fn available_disk_space(path: &Path) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    let mut total = 0_u64;
    let mut free = 0_u64;
    let result =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) };
    (result != 0)
        .then_some(available)
        .ok_or_else(|| "LANDSAT_P4A3_PC_DISK_SPACE_UNAVAILABLE".to_owned())
}

#[cfg(not(windows))]
fn available_disk_space(_path: &Path) -> Result<u64, String> {
    Err("LANDSAT_P4A3_PC_WINDOWS_DISK_PREFLIGHT_REQUIRED".to_owned())
}
