use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, ApplicationServiceError,
    LiveReviewedMpcAssetEvidence, ReviewedMpcCreateAssetTaskRequest,
};
use insar_core::{
    provider_product_key, stable_asset_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    ContractError, DownloadItemProgress, DownloadItemResult, DownloadLayout, DownloadOutcome,
    FrozenSearchBatch, ProviderResult, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest,
};
use insar_download::{
    DownloadWork, DownloadWorker, WorkerControl, PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID,
    PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID,
};
use insar_provider::planetary_computer_asset::{
    PlanetaryComputerResolvedAssetEvidence, PlanetaryComputerReviewedAssetClass,
    MPC_REVIEWED_STAC_IDENTITY_RULE_ID, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use insar_service::LiveAsfSearch;
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

const SOURCE_BATCH_ID: &str = "reviewed-mpc-source-batch";
const S1_ID: &str = "S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2";
const S1_VV_HREF: &str = "https://sentinel1euwest.blob.core.windows.net/s1-grd/GRD/2025/1/23/IW/DV/S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2_887E/measurement/iw-vv.tiff";
const S1_VH_HREF: &str = "https://sentinel1euwest.blob.core.windows.net/s1-grd/GRD/2025/1/23/IW/DV/S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2_887E/measurement/iw-vh.tiff";
const DEM_ID: &str = "Copernicus_DSM_COG_10_N25_00_E121_00_DEM";
const DEM_HREF: &str = "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP30_hh/Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif";

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-reviewed-mpc-{label}-{}-{nonce}.sqlite3",
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
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
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
            finished_at: "2026-09-20T00:00:03Z".to_owned(),
        }
    }
}

struct FixtureEvidence {
    calls: AtomicUsize,
}

impl FixtureEvidence {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl LiveReviewedMpcAssetEvidence for FixtureEvidence {
    fn resolve_reviewed_asset(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        provider_product_id: &str,
        asset_key: &str,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerResolvedAssetEvidence, ContractError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match (class, provider_product_id, asset_key) {
            (PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog, DEM_ID, "data") => {
                Ok(PlanetaryComputerResolvedAssetEvidence {
                    stable_href: DEM_HREF.to_owned(),
                    file_name: format!("{DEM_ID}.tif"),
                    expected_size_bytes: 7_594_159,
                })
            }
            (PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog, S1_ID, "vv") => {
                Ok(PlanetaryComputerResolvedAssetEvidence {
                    stable_href: S1_VV_HREF.to_owned(),
                    file_name: "iw-vv.tiff".to_owned(),
                    expected_size_bytes: 648_045_257,
                })
            }
            (PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog, S1_ID, "vh") => {
                Ok(PlanetaryComputerResolvedAssetEvidence {
                    stable_href: S1_VH_HREF.to_owned(),
                    file_name: "iw-vh.tiff".to_owned(),
                    expected_size_bytes: 603_000_000,
                })
            }
            _ => Err(ContractError::new(
                "FIXTURE_EVIDENCE_MISMATCH",
                "fixture evidence mismatch",
                Map::new(),
            )
            .unwrap()),
        }
    }
}

fn source_item(
    class: PlanetaryComputerReviewedAssetClass,
    product_id: &str,
    asset_keys: &[&str],
) -> SnapshotItem {
    let product_key = provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        class.collection_id(),
        product_id,
    )
    .unwrap();
    let (source_kind, product_kind) = match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => ("sentinel1", "sar_grd"),
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => ("dem", "dem_raster"),
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            panic!("Sentinel-2 bands use the dedicated analysis-band task evidence fixture")
        }
    };
    let assets = asset_keys
        .iter()
        .map(|asset_key| {
            json!({
                "asset_key": asset_key,
                "stable_asset_key": stable_asset_key(&product_key, asset_key).unwrap(),
                "role": "primary",
                "native_roles": ["data"],
                "file_name": null,
                "media_type": "image/tiff; application=geotiff; profile=cloud-optimized",
                "size_bytes": null
            })
        })
        .collect::<Vec<_>>();
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
                Value::String(product_key),
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
            ("assets".to_owned(), Value::Array(assets)),
        ]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed(database: &TempStore, item: SnapshotItem) {
    let mut store = TaskStore::open(&database.0).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: SOURCE_BATCH_ID.to_owned(),
                provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
                created_at: "2026-09-20T00:00:00Z".to_owned(),
                aoi_name: "Reviewed MPC source".to_owned(),
                aoi_geojson: None,
                query: Map::from_iter([
                    (
                        "logical_dataset".to_owned(),
                        item.metadata["logical_dataset"].clone(),
                    ),
                    (
                        "provider_id".to_owned(),
                        Value::String(PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned()),
                    ),
                    (
                        "native_collection_id".to_owned(),
                        item.metadata["native_collection_id"].clone(),
                    ),
                ]),
                search_stats: None,
                candidates: vec![item.clone()],
            },
            workbench: SearchWorkbenchState {
                batch_id: SOURCE_BATCH_ID.to_owned(),
                checked_item_ids: vec![item.item_id],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
}

fn service(database: &TempStore) -> ApplicationService {
    ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(FixtureWorker),
    )
}

fn request(
    item_id: String,
    asset_keys: &[&str],
    task_id: &str,
) -> ReviewedMpcCreateAssetTaskRequest {
    ReviewedMpcCreateAssetTaskRequest {
        request_id: format!("{task_id}-request"),
        source_batch_id: SOURCE_BATCH_ID.to_owned(),
        item_id,
        asset_keys: asset_keys.iter().map(|value| (*value).to_owned()).collect(),
        task_id: task_id.to_owned(),
        output_root: std::env::temp_dir().join("reviewed-mpc-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-09-20T00:00:01Z".to_owned(),
    }
}

fn assert_source_workbench_unchanged(database: &TempStore, expected_item_id: &str) {
    let store = TaskStore::open(&database.0).unwrap();
    let workbench = store.load_workbench().unwrap().unwrap();
    assert_eq!(workbench.batch.batch_id, SOURCE_BATCH_ID);
    assert_eq!(workbench.batch.candidates.len(), 1);
    assert_eq!(workbench.batch.candidates[0].item_id, expected_item_id);
    assert!(workbench.batch.candidates[0].asset_access.is_none());
    assert!(workbench.batch.candidates[0].execution_policy.is_none());
}

#[test]
fn cop30_source_derives_one_executable_asset_task_without_replacing_workbench() {
    let database = TempStore::new("cop30");
    let source = source_item(
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
        DEM_ID,
        &["data"],
    );
    let source_item_id = source.item_id.clone();
    seed(&database, source);
    let service = service(&database);
    let evidence = FixtureEvidence::new();

    service
        .create_reviewed_mpc_asset_task_with_evidence_for_test(
            &request(source_item_id.clone(), &["data"], "mpc-cop30-delivery"),
            &evidence,
        )
        .unwrap();
    assert_eq!(evidence.calls.load(Ordering::SeqCst), 1);
    assert_source_workbench_unchanged(&database, &source_item_id);

    let store = TaskStore::open(&database.0).unwrap();
    let task = store
        .load_task_record("mpc-cop30-delivery")
        .unwrap()
        .expect("derived COP30 task persists");
    assert_ne!(task.snapshot.batch_id, SOURCE_BATCH_ID);
    let batch = store
        .load_search_batch(&task.snapshot.batch_id)
        .unwrap()
        .expect("derived COP30 delivery batch persists");
    assert_eq!(
        batch.query["derived_from_batch_id"],
        Value::String(SOURCE_BATCH_ID.to_owned())
    );
    assert_eq!(batch.candidates.len(), 1);
    let item = &batch.candidates[0];
    let access = item.asset_access.as_ref().unwrap();
    assert_eq!(access.descriptor.stable_href.as_deref(), Some(DEM_HREF));
    assert_eq!(access.expected_size_bytes, Some(7_594_159));
    assert_eq!(
        item.execution_policy.as_ref().unwrap().policy_id,
        PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID
    );
    assert!(!serde_json::to_string(item).unwrap().contains("sig="));

    service.shutdown().unwrap();
}

#[test]
fn sentinel1_source_derives_selected_polarization_assets_through_the_same_boundary() {
    let database = TempStore::new("sentinel1");
    let source = source_item(
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        S1_ID,
        &["vv", "vh"],
    );
    let source_item_id = source.item_id.clone();
    seed(&database, source);
    let service = service(&database);
    let evidence = FixtureEvidence::new();

    service
        .create_reviewed_mpc_asset_task_with_evidence_for_test(
            &request(source_item_id.clone(), &["vv", "vh"], "mpc-s1-grd-delivery"),
            &evidence,
        )
        .unwrap();
    assert_eq!(evidence.calls.load(Ordering::SeqCst), 2);
    assert_source_workbench_unchanged(&database, &source_item_id);

    let store = TaskStore::open(&database.0).unwrap();
    let task = store
        .load_task_record("mpc-s1-grd-delivery")
        .unwrap()
        .expect("derived Sentinel-1 task persists");
    let batch = store
        .load_search_batch(&task.snapshot.batch_id)
        .unwrap()
        .unwrap();
    assert_eq!(batch.candidates.len(), 2);
    let keys = batch
        .candidates
        .iter()
        .map(|item| {
            item.asset_access
                .as_ref()
                .unwrap()
                .reference
                .asset_key
                .as_str()
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, ["vv", "vh"]);
    assert!(batch.candidates.iter().all(|item| {
        item.execution_policy.as_ref().unwrap().policy_id
            == PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID
    }));

    service.shutdown().unwrap();
}

#[test]
fn offline_fixture_marker_cannot_upgrade_to_execution_authority() {
    let database = TempStore::new("offline-fixture-rejected");
    let mut source = source_item(
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
        DEM_ID,
        &["data"],
    );
    source
        .metadata
        .insert("offline_fixture_only".to_owned(), Value::Bool(true));
    let source_item_id = source.item_id.clone();
    seed(&database, source);
    let service = service(&database);
    let evidence = FixtureEvidence::new();

    let error = service
        .create_reviewed_mpc_asset_task_with_evidence_for_test(
            &request(source_item_id, &["data"], "mpc-fixture-must-fail"),
            &evidence,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid
    ));
    assert_eq!(evidence.calls.load(Ordering::SeqCst), 0);
    assert!(TaskStore::open(&database.0)
        .unwrap()
        .load_task_record("mpc-fixture-must-fail")
        .unwrap()
        .is_none());

    service.shutdown().unwrap();
}

#[test]
fn caller_cannot_select_an_unreviewed_asset_key() {
    let database = TempStore::new("asset-key-rejected");
    let source = source_item(
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
        DEM_ID,
        &["data"],
    );
    let source_item_id = source.item_id.clone();
    seed(&database, source);
    let service = service(&database);
    let evidence = FixtureEvidence::new();

    let error = service
        .create_reviewed_mpc_asset_task_with_evidence_for_test(
            &request(source_item_id, &["thumbnail"], "mpc-key-must-fail"),
            &evidence,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid
    ));
    assert_eq!(evidence.calls.load(Ordering::SeqCst), 0);

    service.shutdown().unwrap();
}

#[test]
fn sentinel1_cannot_request_a_reviewed_polarization_missing_from_the_frozen_source() {
    let database = TempStore::new("missing-frozen-polarization");
    let source = source_item(
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
        S1_ID,
        &["vv"],
    );
    let source_item_id = source.item_id.clone();
    seed(&database, source);
    let service = service(&database);
    let evidence = FixtureEvidence::new();

    let error = service
        .create_reviewed_mpc_asset_task_with_evidence_for_test(
            &request(source_item_id, &["vh"], "mpc-missing-pol-must-fail"),
            &evidence,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ApplicationServiceError::ReviewedMpcAssetEvidenceInvalid
    ));
    assert_eq!(evidence.calls.load(Ordering::SeqCst), 0);

    service.shutdown().unwrap();
}
