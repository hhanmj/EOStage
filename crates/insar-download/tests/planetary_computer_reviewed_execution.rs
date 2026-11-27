use std::fs;
use std::io::Read;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use insar_core::{
    provider_product_key, stable_asset_key, AssetRole, DownloadLayout, DownloadOutcome,
    FrozenSearchBatch, FrozenTaskSnapshot, ProviderNetworkPolicy, SearchWorkbenchState,
    SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_download::{
    build_planetary_computer_reviewed_cog_item, evaluate_resolver_cutover_eligibility,
    AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry, AssetResolverRegistry,
    CredentialMaterial, CredentialProvider, CredentialProviderError, DownloadScheduler,
    DownloadWork, DownloadWorker, ProductionDownloadWorker, ResolveContext,
    ResolverCutoverEligibility, SystemClock, TransferOptions, WorkerControl,
    PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID, PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID, PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID,
};
use insar_provider::planetary_computer_asset::{
    validate_planetary_computer_runtime_asset_url, PlanetaryComputerAssetSigner,
    PlanetaryComputerReviewedAssetClass, PlanetaryComputerReviewedAssetEvidence,
    MPC_REVIEWED_STAC_IDENTITY_RULE_ID, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

const S1_ID: &str = "S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2";
const S1_HREF: &str = "https://sentinel1euwest.blob.core.windows.net/s1-grd/GRD/2025/1/23/IW/DV/S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2_887E/measurement/iw-vv.tiff";
const DEM_ID: &str = "Copernicus_DSM_COG_10_N25_00_E121_00_DEM";
const DEM_HREF: &str = "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP30_hh/Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif";
const S2_ID: &str = "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916";
const S2_HREF: &str = "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B02_10m.tif";

fn source_item(
    class: PlanetaryComputerReviewedAssetClass,
    product_id: &str,
    asset_key: &str,
) -> SnapshotItem {
    let product_key = provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        class.collection_id(),
        product_id,
    )
    .unwrap();
    let descriptive_asset_key = stable_asset_key(&product_key, asset_key).unwrap();
    let (source_kind, product_kind) = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => ("sentinel1", "sar_grd"),
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => ("dem", "dem_raster"),
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => ("sentinel2", "optical_scene"),
    };
    SnapshotItem {
        item_id: product_key.clone(),
        provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
        display_name: product_id.to_owned(),
        acquisition_datetime: Some("2025-01-23T21:52:40Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                Value::String(class.logical_dataset().to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String(class.collection_id().to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(class.collection_profile_id().to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String(MPC_REVIEWED_STAC_IDENTITY_RULE_ID.to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(product_id.to_owned()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(product_key.clone()),
            ),
            (
                "stac_item_id".to_owned(),
                Value::String(product_id.to_owned()),
            ),
            (
                "source_kind".to_owned(),
                Value::String(source_kind.to_owned()),
            ),
            (
                "product_kind".to_owned(),
                Value::String(product_kind.to_owned()),
            ),
            (
                "assets".to_owned(),
                json!([{
                    "asset_key": asset_key,
                    "stable_asset_key": descriptive_asset_key,
                    "role": "primary",
                    "native_roles": ["data"],
                    "file_name": null,
                    "media_type": "image/tiff; application=geotiff; profile=cloud-optimized",
                    "size_bytes": null
                }]),
            ),
        ]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn evidence(file_name: &str, size: u64) -> PlanetaryComputerReviewedAssetEvidence {
    PlanetaryComputerReviewedAssetEvidence {
        file_name: file_name.to_owned(),
        expected_size_bytes: size,
    }
}

#[test]
fn reviewed_sar_and_dem_sources_mint_only_exact_resolver_authority() {
    for (class, product_id, asset_key, href, file_name, size, policy_id, resolver_id) in [
        (
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            S1_ID,
            "vv",
            S1_HREF,
            "iw-vv.tiff",
            648_045_257,
            PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID,
            PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
        ),
        (
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
            DEM_ID,
            "data",
            DEM_HREF,
            "Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif",
            7_594_159,
            PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID,
            PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID,
        ),
        (
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
            S2_ID,
            "B02",
            S2_HREF,
            "T48SYC_20260706T032511_B02_10m.tif",
            120_000_000,
            PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID,
            PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID,
        ),
    ] {
        let source = source_item(class, product_id, asset_key);
        assert!(source.asset_access.is_none());
        assert!(source.execution_policy.is_none());

        let executable = build_planetary_computer_reviewed_cog_item(
            &source,
            class,
            asset_key,
            href,
            &evidence(file_name, size),
        )
        .unwrap();
        let policy = executable.execution_policy.as_ref().unwrap();
        let access = executable.asset_access.as_ref().unwrap();
        assert_eq!(policy.policy_id, policy_id);
        assert_eq!(access.descriptor.resolver_id, resolver_id);
        assert_eq!(access.expected_size_bytes, Some(size));
        assert_eq!(access.file_name.as_deref(), Some(file_name));
        assert_eq!(access.role, AssetRole::Primary);
        assert_eq!(executable.item_id, access.reference.stable_asset_key);
        assert!(matches!(
            evaluate_resolver_cutover_eligibility(
                &executable,
                &AsfCollectionProfileRegistry,
                &AssetExecutionPolicyRegistry,
            ),
            ResolverCutoverEligibility::Eligible(_)
        ));
    }
}

#[test]
fn fixture_marker_and_forged_evidence_cannot_mint_execution_authority() {
    let mut fixture = source_item(
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        S1_ID,
        "vv",
    );
    fixture
        .metadata
        .insert("offline_fixture_only".to_owned(), Value::Bool(true));
    assert!(build_planetary_computer_reviewed_cog_item(
        &fixture,
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        "vv",
        S1_HREF,
        &evidence("iw-vv.tiff", 648_045_257),
    )
    .is_err());

    let source = source_item(
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        S1_ID,
        "vv",
    );
    assert!(build_planetary_computer_reviewed_cog_item(
        &source,
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        "vv",
        &format!("{S1_HREF}?sig=forged"),
        &evidence("iw-vv.tiff", 648_045_257),
    )
    .is_err());
    assert!(build_planetary_computer_reviewed_cog_item(
        &source,
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        "vv",
        S1_HREF,
        &evidence("iw-vv.tiff", 0),
    )
    .is_err());
}

#[test]
fn standard_registry_contains_all_exact_reviewed_resolvers() {
    let registry =
        AssetResolverRegistry::with_standard_resolvers_for_network(ProviderNetworkPolicy::direct())
            .unwrap();
    let ids = registry.resolver_ids();
    assert!(ids.contains(&PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID));
    assert!(ids.contains(&PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID));
    assert!(ids.contains(&PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID));
}

struct NoCredentials;

impl CredentialProvider for NoCredentials {
    fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        Err(CredentialProviderError::Missing)
    }
}

#[test]
#[ignore = "requires EOSTAGE_MPC_CROSS_DOMAIN_RESOLVER_LIVE=1; signs fixed reviewed public assets without downloading them"]
fn standard_registry_resolves_reviewed_assets_with_runtime_only_sas() {
    assert_eq!(
        std::env::var("EOSTAGE_MPC_CROSS_DOMAIN_RESOLVER_LIVE").as_deref(),
        Ok("1")
    );
    let registry =
        AssetResolverRegistry::with_standard_resolvers_for_network(ProviderNetworkPolicy::system())
            .unwrap();
    let evidence_resolver = PlanetaryComputerAssetSigner::new(ProviderNetworkPolicy::system())
        .expect("fixed public MPC evidence resolver");
    let clock = SystemClock;
    for (class, product_id, asset_key, expected_href) in [
        (
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            S1_ID,
            "vv",
            S1_HREF,
        ),
        (
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
            DEM_ID,
            "data",
            DEM_HREF,
        ),
    ] {
        let discovered = evidence_resolver
            .resolve_reviewed_asset(class, product_id, asset_key, &|| false)
            .unwrap();
        assert_eq!(discovered.stable_href, expected_href);
        assert!(discovered.expected_size_bytes > 1);
        let frozen_evidence = evidence(&discovered.file_name, discovered.expected_size_bytes);
        let executable = build_planetary_computer_reviewed_cog_item(
            &source_item(class, product_id, asset_key),
            class,
            asset_key,
            &discovered.stable_href,
            &frozen_evidence,
        )
        .unwrap();
        let frozen = executable.asset_access.as_ref().unwrap();
        let resolved = registry
            .resolve_frozen(
                frozen,
                &ResolveContext::new(&NoCredentials, &|| false, &clock),
            )
            .unwrap();
        assert_eq!(
            resolved.suggested_filename(),
            Some(discovered.file_name.as_str())
        );
        assert_eq!(
            resolved.expected_size(),
            Some(discovered.expected_size_bytes)
        );
        assert!(resolved.effective_url().query().is_some());
        validate_planetary_computer_runtime_asset_url(class, resolved.effective_url()).unwrap();
        let debug = format!("{resolved:?}");
        assert!(!debug.contains("sig="));
    }
}

#[test]
#[ignore = "requires EOSTAGE_MPC_COPDEM_TRANSFER_LIVE=1; downloads one reviewed public COP30 COG (~8 MiB) into a temporary directory"]
fn production_worker_downloads_reviewed_cop30_without_persisting_sas() {
    assert_eq!(
        std::env::var("EOSTAGE_MPC_COPDEM_TRANSFER_LIVE").as_deref(),
        Ok("1")
    );
    let class = PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog;
    let evidence_resolver = PlanetaryComputerAssetSigner::new(ProviderNetworkPolicy::system())
        .expect("fixed public MPC evidence resolver");
    let discovered = evidence_resolver
        .resolve_reviewed_asset(class, DEM_ID, "data", &|| false)
        .expect("exact public COP30 asset evidence");
    assert_eq!(discovered.stable_href, DEM_HREF);
    assert!(discovered.expected_size_bytes > 1);

    let item = build_planetary_computer_reviewed_cog_item(
        &source_item(class, DEM_ID, "data"),
        class,
        "data",
        &discovered.stable_href,
        &evidence(&discovered.file_name, discovered.expected_size_bytes),
    )
    .expect("reviewed COP30 execution authority");
    let frozen_json = serde_json::to_string(&item).unwrap();
    assert!(!frozen_json.contains("sig="));
    assert!(!frozen_json.contains("msft:expiry"));

    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output_root = std::env::temp_dir().join(format!(
        "sceneport-mpc-cop30-transfer-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&output_root).unwrap();

    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": "mpc-cop30-live-transfer",
        "batch_id": "mpc-cop30-live-batch",
        "provider_id": PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        "created_at": "2026-09-20T00:00:00Z",
        "aoi_name": "MPC COP30 live transfer",
        "aoi_geojson": null,
        "query": {
            "logical_dataset": class.logical_dataset(),
            "native_collection_id": class.collection_id()
        },
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id.clone()],
        "output_root": output_root,
        "layout": "direct"
    }))
    .unwrap();

    let worker = ProductionDownloadWorker::new(TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(90),
        chunk_size: 256 * 1024,
        ..TransferOptions::default()
    })
    .unwrap();
    let result = worker.execute(
        DownloadWork { task, item },
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert!(result.failure.is_none());
    assert_eq!(result.bytes_written, discovered.expected_size_bytes);
    assert!(result.destination.is_file());
    assert_eq!(
        fs::metadata(&result.destination).unwrap().len(),
        discovered.expected_size_bytes
    );
    let mut file = fs::File::open(&result.destination).unwrap();
    let mut prefix = [0_u8; 4];
    file.read_exact(&mut prefix).unwrap();
    assert!(
        prefix == *b"II*\0" || prefix == *b"MM\0*",
        "COP30 output must retain TIFF magic"
    );

    fs::remove_dir_all(&output_root).ok();
}

#[test]
#[ignore = "requires EOSTAGE_MPC_COPDEM_SCHEDULER_LIVE=1; downloads one reviewed public COP30 COG and verifies persisted task recovery"]
fn scheduler_persists_reviewed_cop30_completion_across_store_reopen() {
    assert_eq!(
        std::env::var("EOSTAGE_MPC_COPDEM_SCHEDULER_LIVE").as_deref(),
        Ok("1")
    );
    let class = PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog;
    let evidence_resolver = PlanetaryComputerAssetSigner::new(ProviderNetworkPolicy::system())
        .expect("fixed public MPC evidence resolver");
    let discovered = evidence_resolver
        .resolve_reviewed_asset(class, DEM_ID, "data", &|| false)
        .expect("exact public COP30 asset evidence");
    let item = build_planetary_computer_reviewed_cog_item(
        &source_item(class, DEM_ID, "data"),
        class,
        "data",
        &discovered.stable_href,
        &evidence(&discovered.file_name, discovered.expected_size_bytes),
    )
    .expect("reviewed COP30 execution authority");

    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sceneport-mpc-cop30-scheduler-{}-{nonce}",
        std::process::id()
    ));
    let output_root = root.join("output");
    let database = root.join("task-store.sqlite3");
    fs::create_dir_all(&output_root).unwrap();

    const BATCH_ID: &str = "mpc-cop30-scheduler-live-batch";
    const TASK_ID: &str = "mpc-cop30-scheduler-live-task";
    {
        let mut store = TaskStore::open(&database).expect("open COP30 task store");
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: BATCH_ID.to_owned(),
                    provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
                    created_at: "2026-09-20T00:00:00Z".to_owned(),
                    aoi_name: "MPC COP30 scheduler live".to_owned(),
                    aoi_geojson: None,
                    query: Map::from_iter([
                        (
                            "logical_dataset".to_owned(),
                            Value::String(class.logical_dataset().to_owned()),
                        ),
                        (
                            "native_collection_id".to_owned(),
                            Value::String(class.collection_id().to_owned()),
                        ),
                    ]),
                    search_stats: None,
                    candidates: vec![item.clone()],
                },
                workbench: SearchWorkbenchState {
                    batch_id: BATCH_ID.to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .expect("save COP30 executable batch");
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: TASK_ID.to_owned(),
                batch_id: BATCH_ID.to_owned(),
                selected_item_ids: vec![item.item_id.clone()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-09-20T00:00:01Z".to_owned(),
            })
            .expect("create COP30 task");
    }

    let worker = std::sync::Arc::new(
        ProductionDownloadWorker::new(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(90),
            chunk_size: 256 * 1024,
            ..TransferOptions::default()
        })
        .unwrap(),
    );
    let scheduler =
        DownloadScheduler::start(&database, worker, Duration::from_millis(20)).expect("scheduler");
    scheduler.wake();

    let deadline = Instant::now() + Duration::from_secs(45);
    let completed = loop {
        let summary = TaskStore::open(&database)
            .expect("reopen COP30 store while running")
            .core_restore(false)
            .expect("restore COP30 task summary")
            .tasks
            .into_iter()
            .find(|task| task.task_id == TASK_ID)
            .expect("COP30 task summary");
        match summary.status {
            TaskLifecycleStatus::Completed => break summary,
            TaskLifecycleStatus::Failed | TaskLifecycleStatus::Cancelled => {
                panic!(
                    "COP30 scheduler task became terminal unexpectedly: status={:?}, bytes={}",
                    summary.status, summary.transferred_bytes
                );
            }
            _ => {
                assert!(Instant::now() < deadline, "COP30 scheduler task timed out");
                thread::sleep(Duration::from_millis(50));
            }
        }
    };
    scheduler.shutdown();

    assert_eq!(completed.transferred_bytes, discovered.expected_size_bytes);
    let reopened = TaskStore::open(&database)
        .expect("reopen completed COP30 store")
        .core_restore(false)
        .expect("restore completed COP30 task")
        .tasks
        .into_iter()
        .find(|task| task.task_id == TASK_ID)
        .expect("completed COP30 task persists");
    assert_eq!(reopened.status, TaskLifecycleStatus::Completed);
    assert_eq!(reopened.transferred_bytes, discovered.expected_size_bytes);

    let destination = output_root.join(&discovered.file_name);
    assert!(destination.is_file());
    assert_eq!(
        fs::metadata(&destination).unwrap().len(),
        discovered.expected_size_bytes
    );
    fs::remove_dir_all(&root).ok();
}

#[test]
#[ignore = "requires EOSTAGE_MPC_SENTINEL1_CANCEL_LIVE=1; starts one reviewed public Sentinel-1 GRD COG task and cancels it before full-scene completion"]
fn scheduler_cancels_reviewed_sentinel1_grd_without_promoting_partial_output() {
    assert_eq!(
        std::env::var("EOSTAGE_MPC_SENTINEL1_CANCEL_LIVE").as_deref(),
        Ok("1")
    );
    let class = PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog;
    let evidence_resolver = PlanetaryComputerAssetSigner::new(ProviderNetworkPolicy::system())
        .expect("fixed public MPC evidence resolver");
    let discovered = evidence_resolver
        .resolve_reviewed_asset(class, S1_ID, "vv", &|| false)
        .expect("exact public Sentinel-1 GRD asset evidence");
    assert_eq!(discovered.stable_href, S1_HREF);
    assert!(discovered.expected_size_bytes > 100 * 1024 * 1024);

    let item = build_planetary_computer_reviewed_cog_item(
        &source_item(class, S1_ID, "vv"),
        class,
        "vv",
        &discovered.stable_href,
        &evidence(&discovered.file_name, discovered.expected_size_bytes),
    )
    .expect("reviewed Sentinel-1 GRD execution authority");

    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sceneport-mpc-s1-cancel-{}-{nonce}",
        std::process::id()
    ));
    let output_root = root.join("output");
    let database = root.join("task-store.sqlite3");
    fs::create_dir_all(&output_root).unwrap();

    const BATCH_ID: &str = "mpc-s1-cancel-live-batch";
    const TASK_ID: &str = "mpc-s1-cancel-live-task";
    {
        let mut store = TaskStore::open(&database).expect("open Sentinel-1 task store");
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: BATCH_ID.to_owned(),
                    provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
                    created_at: "2026-09-20T00:00:00Z".to_owned(),
                    aoi_name: "MPC Sentinel-1 cancel live".to_owned(),
                    aoi_geojson: None,
                    query: Map::from_iter([
                        (
                            "logical_dataset".to_owned(),
                            Value::String(class.logical_dataset().to_owned()),
                        ),
                        (
                            "native_collection_id".to_owned(),
                            Value::String(class.collection_id().to_owned()),
                        ),
                    ]),
                    search_stats: None,
                    candidates: vec![item.clone()],
                },
                workbench: SearchWorkbenchState {
                    batch_id: BATCH_ID.to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .expect("save Sentinel-1 executable batch");
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: TASK_ID.to_owned(),
                batch_id: BATCH_ID.to_owned(),
                selected_item_ids: vec![item.item_id.clone()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-09-20T00:00:01Z".to_owned(),
            })
            .expect("create Sentinel-1 task");
    }

    let worker = std::sync::Arc::new(
        ProductionDownloadWorker::new(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(90),
            chunk_size: 64 * 1024,
            ..TransferOptions::default()
        })
        .unwrap(),
    );
    let scheduler =
        DownloadScheduler::start(&database, worker, Duration::from_millis(10)).expect("scheduler");
    scheduler.wake();

    let start_deadline = Instant::now() + Duration::from_secs(20);
    let before_cancel = loop {
        let summary = TaskStore::open(&database)
            .expect("reopen Sentinel-1 store while running")
            .core_restore(false)
            .expect("restore Sentinel-1 task summary")
            .tasks
            .into_iter()
            .find(|task| task.task_id == TASK_ID)
            .expect("Sentinel-1 task summary");
        if summary.status == TaskLifecycleStatus::Running && summary.transferred_bytes > 0 {
            break summary;
        }
        if matches!(
            summary.status,
            TaskLifecycleStatus::Completed
                | TaskLifecycleStatus::Failed
                | TaskLifecycleStatus::Cancelled
        ) {
            panic!(
                "Sentinel-1 task became terminal before cancellation gate: status={:?}, bytes={}",
                summary.status, summary.transferred_bytes
            );
        }
        assert!(
            Instant::now() < start_deadline,
            "Sentinel-1 task did not begin bounded transfer"
        );
        thread::sleep(Duration::from_millis(20));
    };

    {
        let mut store = TaskStore::open(&database).expect("open Sentinel-1 store for cancellation");
        let task = store
            .load_task(TASK_ID)
            .expect("load Sentinel-1 task")
            .expect("Sentinel-1 task remains actionable");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: TASK_ID.to_owned(),
                expected_revision: task.runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::CancelTask,
                    item_ids: vec![],
                },
                event_id: "mpc-s1-cancel-live:cancel".to_owned(),
                occurred_at: "2026-09-20T00:00:02Z".to_owned(),
            })
            .expect("persist Sentinel-1 cancellation");
    }
    scheduler.wake();

    let cancel_deadline = Instant::now() + Duration::from_secs(20);
    let cancelled = loop {
        let summary = TaskStore::open(&database)
            .expect("reopen cancelled Sentinel-1 store")
            .core_restore(false)
            .expect("restore cancelled Sentinel-1 task")
            .tasks
            .into_iter()
            .find(|task| task.task_id == TASK_ID)
            .expect("cancelled Sentinel-1 task summary");
        if summary.status == TaskLifecycleStatus::Cancelled {
            break summary;
        }
        if matches!(
            summary.status,
            TaskLifecycleStatus::Completed | TaskLifecycleStatus::Failed
        ) {
            panic!(
                "Sentinel-1 cancellation produced wrong terminal state: status={:?}, bytes={}",
                summary.status, summary.transferred_bytes
            );
        }
        assert!(
            Instant::now() < cancel_deadline,
            "Sentinel-1 task did not reach cancelled state"
        );
        thread::sleep(Duration::from_millis(20));
    };
    scheduler.shutdown();

    assert!(before_cancel.transferred_bytes > 0);
    assert!(cancelled.transferred_bytes < discovered.expected_size_bytes);
    assert!(!output_root.join(&discovered.file_name).exists());

    let reopened = TaskStore::open(&database)
        .expect("reopen cancelled Sentinel-1 store after scheduler shutdown")
        .core_restore(false)
        .expect("restore cancelled Sentinel-1 task after restart")
        .tasks
        .into_iter()
        .find(|task| task.task_id == TASK_ID)
        .expect("cancelled Sentinel-1 task persists");
    assert_eq!(reopened.status, TaskLifecycleStatus::Cancelled);
    assert!(reopened.transferred_bytes < discovered.expected_size_bytes);

    fs::remove_dir_all(&root).ok();
}
